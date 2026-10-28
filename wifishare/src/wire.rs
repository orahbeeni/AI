//! Encrypted, authenticated links between paired machines.
//!
//! A TCP connection starts with a 5-byte preamble (magic + purpose), then a Noise XX handshake
//! that proves both sides hold their long-term keys. After that every frame is
//! `u16 length | ciphertext`, and each plaintext is `flag | data` so messages larger than one
//! Noise frame (64 KiB) span several frames.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde::de::DeserializeOwned;
use snow::StatelessTransportState;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::config::Identity;
use crate::proto::Msg;
use crate::status::Rtt;

pub const NOISE_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const PROLOGUE: &[u8] = b"wifishare/1";
const MAGIC: &[u8; 4] = b"WSH1";
const MAX_FRAME: usize = 65535;
const TAG_LEN: usize = 16;
const CHUNK: usize = MAX_FRAME - TAG_LEN - 1;
const MAX_MESSAGE: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Input events, clipboard notices and heartbeats.
    Control = 1,
    /// One large clipboard payload or file set.
    Bulk = 2,
    /// First contact between two machines that have not been paired yet.
    Pair = 3,
}

impl Purpose {
    fn from_byte(b: u8) -> Option<Purpose> {
        match b {
            1 => Some(Purpose::Control),
            2 => Some(Purpose::Bulk),
            3 => Some(Purpose::Pair),
            _ => None,
        }
    }
}

pub struct Secured {
    pub reader: FrameReader,
    pub writer: FrameWriter,
    pub remote_key: Vec<u8>,
    /// Noise handshake hash; both sides derive the pairing code from it.
    pub hash: Vec<u8>,
    pub peer_addr: SocketAddr,
    pub local_addr: SocketAddr,
}

/// Dials `addr` and runs the initiator side of the handshake.
pub async fn connect(addr: SocketAddr, id: &Identity, purpose: Purpose, timeout: Duration) -> Result<Secured> {
    let mut stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| anyhow::anyhow!("connecting to {addr} timed out"))?
        .with_context(|| format!("connecting to {addr}"))?;
    tune(&stream, purpose);
    tokio::time::timeout(timeout, async {
        let mut preamble = MAGIC.to_vec();
        preamble.push(purpose as u8);
        stream.write_all(&preamble).await?;
        let mut hs = snow::Builder::new(NOISE_PARAMS.parse()?).local_private_key(&id.private)?.prologue(PROLOGUE)?.build_initiator()?;
        let mut buf = vec![0u8; MAX_FRAME];
        let n = hs.write_message(&[], &mut buf)?;
        write_raw(&mut stream, &buf[..n]).await?;
        let msg = read_raw(&mut stream).await?;
        hs.read_message(&msg, &mut buf)?;
        let n = hs.write_message(&[], &mut buf)?;
        write_raw(&mut stream, &buf[..n]).await?;
        finish(stream, hs)
    })
    .await
    .map_err(|_| anyhow::anyhow!("handshake with {addr} timed out"))?
}

/// Runs the responder side of the handshake on an accepted connection.
pub async fn accept(mut stream: TcpStream, id: &Identity, timeout: Duration) -> Result<(Purpose, Secured)> {
    tokio::time::timeout(timeout, async {
        let mut preamble = [0u8; 5];
        stream.read_exact(&mut preamble).await?;
        if &preamble[..4] != MAGIC {
            bail!("not a wifishare peer");
        }
        let purpose = Purpose::from_byte(preamble[4]).context("unknown connection purpose")?;
        tune(&stream, purpose);
        let mut hs = snow::Builder::new(NOISE_PARAMS.parse()?).local_private_key(&id.private)?.prologue(PROLOGUE)?.build_responder()?;
        let mut buf = vec![0u8; MAX_FRAME];
        let msg = read_raw(&mut stream).await?;
        hs.read_message(&msg, &mut buf)?;
        let n = hs.write_message(&[], &mut buf)?;
        write_raw(&mut stream, &buf[..n]).await?;
        let msg = read_raw(&mut stream).await?;
        hs.read_message(&msg, &mut buf)?;
        Ok((purpose, finish(stream, hs)?))
    })
    .await
    .map_err(|_| anyhow::anyhow!("handshake timed out"))?
}

fn finish(stream: TcpStream, hs: snow::HandshakeState) -> Result<Secured> {
    let remote_key = hs.get_remote_static().context("peer sent no static key")?.to_vec();
    let hash = hs.get_handshake_hash().to_vec();
    let noise = Arc::new(hs.into_stateless_transport_mode()?);
    let peer_addr = stream.peer_addr()?;
    let local_addr = stream.local_addr()?;
    let (rd, wr) = stream.into_split();
    Ok(Secured {
        reader: FrameReader { rd, noise: noise.clone(), nonce: 0, frame: vec![0; MAX_FRAME], plain: vec![0; MAX_FRAME] },
        writer: FrameWriter { wr, noise, nonce: 0, out: Vec::new(), plain: Vec::with_capacity(MAX_FRAME) },
        remote_key,
        hash,
        peer_addr,
        local_addr,
    })
}

/// Low latency for input, a high-priority Wi-Fi queue (DSCP EF maps to WMM voice), and
/// keepalives as a backstop for the application heartbeat.
fn tune(stream: &TcpStream, purpose: Purpose) {
    let _ = stream.set_nodelay(true);
    let sock = socket2::SockRef::from(stream);
    if purpose != Purpose::Bulk {
        let _ = sock.set_tos_v4(0xB8);
    }
    let keepalive = socket2::TcpKeepalive::new().with_time(Duration::from_secs(5)).with_interval(Duration::from_secs(2));
    #[cfg(not(windows))]
    let keepalive = keepalive.with_retries(3);
    let _ = sock.set_tcp_keepalive(&keepalive);
    #[cfg(target_os = "linux")]
    let _ = sock.set_tcp_user_timeout(Some(Duration::from_secs(10)));
}

async fn write_raw(stream: &mut TcpStream, data: &[u8]) -> Result<()> {
    let mut out = (data.len() as u16).to_be_bytes().to_vec();
    out.extend_from_slice(data);
    stream.write_all(&out).await?;
    Ok(())
}

async fn read_raw(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let len = stream.read_u16().await? as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

pub struct FrameReader {
    rd: OwnedReadHalf,
    noise: Arc<StatelessTransportState>,
    nonce: u64,
    frame: Vec<u8>,
    plain: Vec<u8>,
}

impl FrameReader {
    pub async fn recv<T: DeserializeOwned>(&mut self) -> Result<T> {
        let mut message = Vec::new();
        loop {
            let len = self.rd.read_u16().await? as usize;
            self.rd.read_exact(&mut self.frame[..len]).await?;
            let n = self.noise.read_message(self.nonce, &self.frame[..len], &mut self.plain).map_err(|e| anyhow::anyhow!("decrypting: {e}"))?;
            self.nonce += 1;
            if n == 0 {
                bail!("empty frame");
            }
            message.extend_from_slice(&self.plain[1..n]);
            if message.len() > MAX_MESSAGE {
                bail!("message too large");
            }
            if self.plain[0] == 0 {
                break;
            }
        }
        Ok(postcard::from_bytes(&message)?)
    }
}

pub struct FrameWriter {
    wr: OwnedWriteHalf,
    noise: Arc<StatelessTransportState>,
    nonce: u64,
    out: Vec<u8>,
    plain: Vec<u8>,
}

impl FrameWriter {
    /// Encrypts `msg` into the output buffer; `flush` sends everything queued in one write.
    pub fn queue<T: Serialize>(&mut self, msg: &T) -> Result<()> {
        let bytes = postcard::to_stdvec(msg)?;
        let mut chunks = bytes.chunks(CHUNK).peekable();
        if chunks.peek().is_none() {
            return self.queue_frame(&[], true);
        }
        while let Some(chunk) = chunks.next() {
            self.queue_frame(chunk, chunks.peek().is_none())?;
        }
        Ok(())
    }

    fn queue_frame(&mut self, data: &[u8], last: bool) -> Result<()> {
        self.plain.clear();
        self.plain.push(if last { 0 } else { 1 });
        self.plain.extend_from_slice(data);
        let start = self.out.len();
        self.out.resize(start + 2 + self.plain.len() + TAG_LEN, 0);
        let n = self.noise.write_message(self.nonce, &self.plain, &mut self.out[start + 2..]).map_err(|e| anyhow::anyhow!("encrypting: {e}"))?;
        self.nonce += 1;
        self.out[start..start + 2].copy_from_slice(&(n as u16).to_be_bytes());
        self.out.truncate(start + 2 + n);
        Ok(())
    }

    pub async fn flush(&mut self) -> Result<()> {
        if !self.out.is_empty() {
            self.wr.write_all(&self.out).await?;
            self.out.clear();
        }
        Ok(())
    }

    pub async fn send<T: Serialize>(&mut self, msg: &T) -> Result<()> {
        self.queue(msg)?;
        self.flush().await
    }
}

/// A control link: messages in and out over channels, with heartbeats handled internally.
/// `rx` yields `None` once the link is dead (peer gone, silent too long, or write failure).
pub struct Link {
    pub rx: mpsc::UnboundedReceiver<Msg>,
    pub tx: mpsc::UnboundedSender<Msg>,
    pub rtt: Rtt,
    tasks: [JoinHandle<()>; 2],
}

impl Drop for Link {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

impl Link {
    pub fn spawn(reader: FrameReader, writer: FrameWriter, heartbeat: Duration, timeout: Duration, label: String) -> Link {
        let (in_tx, in_rx) = mpsc::unbounded_channel();
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        let (dead_tx, dead_rx) = watch::channel(false);
        let dead_tx = Arc::new(dead_tx);
        let (start, rtt) = (Instant::now(), Rtt::default());
        let reader_task = tokio::spawn(read_loop(reader, in_tx, out_tx.clone(), timeout, label.clone(), (start, rtt.clone()), dead_tx.clone(), dead_rx.clone()));
        let writer_task = tokio::spawn(write_loop(writer, out_rx, heartbeat, label, start, dead_tx, dead_rx));
        Link { rx: in_rx, tx: out_tx, rtt, tasks: [reader_task, writer_task] }
    }
}

async fn read_loop(
    mut reader: FrameReader,
    in_tx: mpsc::UnboundedSender<Msg>,
    out_tx: mpsc::UnboundedSender<Msg>,
    timeout: Duration,
    label: String,
    (start, rtt): (Instant, Rtt),
    dead_tx: Arc<watch::Sender<bool>>,
    mut dead_rx: watch::Receiver<bool>,
) {
    loop {
        let msg = tokio::select! {
            _ = dead_rx.changed() => break,
            r = tokio::time::timeout(timeout, reader.recv::<Msg>()) => r,
        };
        match msg {
            Err(_) => {
                log::warn!("{label}: silent for {:.1}s, dropping the link", timeout.as_secs_f64());
                break;
            }
            Ok(Err(e)) => {
                log::info!("{label}: link closed ({e:#})");
                break;
            }
            Ok(Ok(Msg::Ping(n))) => {
                let _ = out_tx.send(Msg::Pong(n));
            }
            Ok(Ok(Msg::Pong(sent))) => rtt.set(start.elapsed().as_millis().saturating_sub(sent as u128) as u32),
            Ok(Ok(m)) => {
                if in_tx.send(m).is_err() {
                    break;
                }
            }
        }
    }
    dead_tx.send_replace(true);
}

async fn write_loop(
    mut writer: FrameWriter,
    mut out_rx: mpsc::UnboundedReceiver<Msg>,
    heartbeat: Duration,
    label: String,
    start: Instant,
    dead_tx: Arc<watch::Sender<bool>>,
    mut dead_rx: watch::Receiver<bool>,
) {
    let mut tick = tokio::time::interval(heartbeat);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut batch = Vec::new();
    loop {
        tokio::select! {
            _ = dead_rx.changed() => break,
            _ = tick.tick() => batch.push(Msg::Ping(start.elapsed().as_millis() as u64)),
            m = out_rx.recv() => match m {
                Some(m) => {
                    batch.push(m);
                    while let Ok(m) = out_rx.try_recv() {
                        batch.push(m);
                    }
                }
                None => break,
            },
        }
        // Only the latest absolute position matters; a backlog of moves would just replay lag.
        let mut ok = true;
        for i in 0..batch.len() {
            let superseded = matches!(batch[i], Msg::Motion { .. }) && matches!(batch.get(i + 1), Some(Msg::Motion { .. }));
            if !superseded && writer.queue(&batch[i]).is_err() {
                ok = false;
            }
        }
        batch.clear();
        if !ok || writer.flush().await.is_err() {
            log::info!("{label}: write failed, dropping the link");
            break;
        }
    }
    dead_tx.send_replace(true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Identity;
    use crate::proto::Hello;

    fn identity() -> Identity {
        let pair = snow::Builder::new(NOISE_PARAMS.parse().unwrap()).generate_keypair().unwrap();
        Identity { private: pair.private, public: pair.public }
    }

    #[tokio::test]
    async fn handshake_authenticates_and_carries_large_messages() {
        let (a, b) = (identity(), identity());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let b2 = b.clone();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (purpose, mut s) = accept(stream, &b2, Duration::from_secs(5)).await.unwrap();
            assert_eq!(purpose, Purpose::Bulk);
            let big: Vec<u8> = s.reader.recv().await.unwrap();
            s.writer.send(&(big.len() as u64)).await.unwrap();
            (s.remote_key, s.hash)
        });
        let mut c = connect(addr, &a, Purpose::Bulk, Duration::from_secs(5)).await.unwrap();
        let big: Vec<u8> = (0..300_000u32).map(|i| i as u8).collect();
        c.writer.send(&big).await.unwrap();
        let len: u64 = c.reader.recv().await.unwrap();
        assert_eq!(len, 300_000);
        let (seen_key, hash) = server.await.unwrap();
        assert_eq!(seen_key, a.public);
        assert_eq!(c.remote_key, b.public);
        assert_eq!(hash, c.hash, "both sides derive the same pairing code");
    }

    #[tokio::test]
    async fn silent_peer_is_dropped() {
        let (a, b) = (identity(), identity());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // The far end completes the handshake and then never says anything, like a peer that
        // walked out of Wi-Fi range without closing the socket.
        let _server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let s = accept(stream, &b, Duration::from_secs(5)).await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(s);
        });
        let c = connect(addr, &a, Purpose::Control, Duration::from_secs(5)).await.unwrap();
        let mut link = Link::spawn(c.reader, c.writer, Duration::from_millis(100), Duration::from_millis(400), "test".into());
        let t = Instant::now();
        link.tx.send(Msg::Hello(Hello { version: 1, name: "x".into(), os: crate::proto::Os::Linux, screen: None })).unwrap();
        assert!(link.rx.recv().await.is_none());
        assert!(t.elapsed() < Duration::from_secs(2), "took {:?}", t.elapsed());
    }
}
