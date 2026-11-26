//! tokio drivers that run [`SinkSession`] / [`SourceSession`] over TCP.

use std::io;
use std::time::Instant;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{debug, info, trace, warn};

use crate::rtsp::{Decoder, Message};
use crate::session::{Output, SessionEvent, SinkConfig, SinkSession, SinkState};
use crate::source::{SourceConfig, SourceEvent, SourceOutput, SourceSession, SourceState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkCommand {
    RequestIdr,
    Pause,
    Play,
    Teardown,
}

async fn write_msg(wr: &mut tokio::net::tcp::OwnedWriteHalf, m: &Message, who: &str) -> io::Result<()> {
    debug!("{who} -> {}", m.summary());
    trace!("{who} -> {m}");
    wr.write_all(&m.to_bytes()).await
}

/// Runs a sink RTSP session until it closes. Events (including the final
/// [`SessionEvent::Closed`]) go to `events`. Dropping the command sender
/// tears the session down.
pub async fn run_sink(
    stream: TcpStream,
    cfg: SinkConfig,
    events: mpsc::UnboundedSender<SessionEvent>,
    mut commands: mpsc::UnboundedReceiver<SinkCommand>,
) -> io::Result<()> {
    let _ = stream.set_nodelay(true);
    let peer = stream.peer_addr().ok();
    let (mut rd, mut wr) = stream.into_split();
    let mut session = SinkSession::new(cfg, Instant::now());
    let mut dec = Decoder::new();
    let mut buf = vec![0u8; 16 * 1024];
    let mut commands_open = true;
    info!(?peer, "RTSP session started");

    loop {
        let deadline = session.deadline().map(tokio::time::Instant::from_std);
        let outputs: Vec<Output> = tokio::select! {
            n = rd.read(&mut buf) => match n {
                Ok(0) => session.on_disconnected(),
                Err(e) => {
                    warn!("RTSP read error: {e}");
                    session.on_disconnected()
                }
                Ok(n) => {
                    dec.feed(&buf[..n]);
                    let mut outs = Vec::new();
                    loop {
                        match dec.next_message() {
                            Ok(Some(m)) => {
                                debug!("sink <- {}", m.summary());
                                trace!("sink <- {m}");
                                outs.extend(session.handle(m, Instant::now()));
                            }
                            Ok(None) => break,
                            Err(e) => {
                                warn!("RTSP parse error: {e}");
                                outs.extend(session.on_disconnected());
                                break;
                            }
                        }
                    }
                    outs
                }
            },
            cmd = commands.recv(), if commands_open => match cmd {
                Some(SinkCommand::RequestIdr) => session.request_idr(),
                Some(SinkCommand::Pause) => session.pause(),
                Some(SinkCommand::Play) => session.play(),
                Some(SinkCommand::Teardown) => session.teardown(Instant::now()),
                None => {
                    commands_open = false;
                    session.teardown(Instant::now())
                }
            },
            _ = async { tokio::time::sleep_until(deadline.unwrap()).await }, if deadline.is_some() => {
                session.on_timer(Instant::now())
            }
        };
        for o in outputs {
            match o {
                Output::Send(m) => {
                    if let Err(e) = write_msg(&mut wr, &m, "sink").await {
                        warn!("RTSP write error: {e}");
                        for o in session.on_disconnected() {
                            if let Output::Event(ev) = o {
                                let _ = events.send(ev);
                            }
                        }
                        return Err(e);
                    }
                }
                Output::Event(ev) => {
                    debug!(?ev, "session event");
                    let _ = events.send(ev);
                }
            }
        }
        if session.state() == SinkState::Closed {
            let _ = wr.shutdown().await;
            return Ok(());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceCommand {
    Teardown,
}

/// Runs a simulated source session on an accepted sink connection.
pub async fn run_source(
    stream: TcpStream,
    cfg: SourceConfig,
    events: mpsc::UnboundedSender<SourceEvent>,
    mut commands: mpsc::UnboundedReceiver<SourceCommand>,
) -> io::Result<()> {
    let _ = stream.set_nodelay(true);
    let (mut rd, mut wr) = stream.into_split();
    let mut session = SourceSession::new(cfg, Instant::now());
    let mut dec = Decoder::new();
    let mut buf = vec![0u8; 16 * 1024];
    let mut commands_open = true;

    let mut outputs = session.start();
    loop {
        for o in outputs.drain(..) {
            match o {
                SourceOutput::Send(m) => write_msg(&mut wr, &m, "source").await?,
                SourceOutput::Event(ev) => {
                    let closed = matches!(ev, SourceEvent::Closed { .. });
                    let _ = events.send(ev);
                    if closed {
                        // let the last response flush before closing
                        let _ = wr.flush().await;
                    }
                }
            }
        }
        if session.state() == SourceState::Closed {
            let _ = wr.shutdown().await;
            return Ok(());
        }
        let ka = session.next_keepalive().map(tokio::time::Instant::from_std);
        outputs = tokio::select! {
            n = rd.read(&mut buf) => match n {
                Ok(0) | Err(_) => {
                    let _ = events.send(SourceEvent::Closed { reason: "sink disconnected".into() });
                    return Ok(());
                }
                Ok(n) => {
                    dec.feed(&buf[..n]);
                    let mut outs = Vec::new();
                    while let Ok(Some(m)) = dec.next_message() {
                        debug!("source <- {}", m.summary());
                        outs.extend(session.handle(m, Instant::now()));
                    }
                    outs
                }
            },
            cmd = commands.recv(), if commands_open => match cmd {
                Some(SourceCommand::Teardown) => session.trigger_teardown(),
                None => {
                    commands_open = false;
                    session.trigger_teardown()
                }
            },
            _ = async { tokio::time::sleep_until(ka.unwrap()).await }, if ka.is_some() => session.on_timer(Instant::now()),
        };
    }
}
