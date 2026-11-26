//! "Simulated phone": a Wi-Fi Display source on localhost. It runs the source
//! side of RTSP (wfd-core), streams a GStreamer test picture as RTP/MPEG-TS
//! (wfd-media) and listens for UIBC, writing the input it receives onto the
//! picture, so the whole receiver can be checked without a phone.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing::{info, warn};
use wfd_core::driver::{SourceCommand, run_source};
use wfd_core::source::{SourceConfig, SourceEvent};
use wfd_core::uibc::parse_packet;
use wfd_media::{TestSource, TestSourceConfig};

pub type LogFn = Arc<dyn Fn(String) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct SimOptions {
    pub max_width: u32,
    pub max_height: u32,
    pub max_fps: u32,
    pub uibc: bool,
}

impl Default for SimOptions {
    fn default() -> Self {
        Self { max_width: 1920, max_height: 1080, max_fps: 30, uibc: true }
    }
}

/// Everything the simulated phone received over UIBC (for tests).
pub type InputLog = Arc<Mutex<Vec<String>>>;

pub struct SimPhone {
    pub rtsp_addr: SocketAddr,
    pub inputs: InputLog,
    commands: mpsc::UnboundedSender<SourceCommand>,
}

const CAPTION: &str = "Simulated phone";

impl SimPhone {
    pub async fn start(opts: SimOptions, log: LogFn) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let rtsp_addr = listener.local_addr()?;
        let uibc = if opts.uibc { Some(TcpListener::bind("127.0.0.1:0").await?) } else { None };
        let uibc_port = uibc.as_ref().and_then(|l| l.local_addr().ok()).map(|a| a.port());
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let media: Arc<Mutex<Option<TestSource>>> = Arc::new(Mutex::new(None));
        let inputs: InputLog = Arc::new(Mutex::new(Vec::new()));

        if let Some(ul) = uibc {
            let media = media.clone();
            let inputs = inputs.clone();
            let log = log.clone();
            tokio::spawn(async move {
                let Ok(Ok((mut s, _))) = tokio::time::timeout(Duration::from_secs(60), ul.accept()).await else { return };
                log("simulated phone: input back-channel connected".into());
                let mut buf = Vec::new();
                let mut chunk = [0u8; 2048];
                loop {
                    let n = match s.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    buf.extend_from_slice(&chunk[..n]);
                    while let Some((p, used)) = parse_packet(&buf) {
                        buf.drain(..used);
                        if let Some(p) = p {
                            let d = p.describe();
                            if let Some(m) = media.lock().unwrap().as_ref() {
                                m.set_caption(&format!("{CAPTION} - input: {d}"));
                            }
                            inputs.lock().unwrap().push(d);
                        }
                    }
                }
            });
        }

        let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
        let cfg = SourceConfig {
            host: "127.0.0.1".into(),
            max_width: opts.max_width,
            max_height: opts.max_height,
            max_fps: opts.max_fps,
            prefer_aac: true,
            uibc_port,
            session_timeout: 30,
            keepalive_interval: Duration::from_secs(20),
        };
        tokio::spawn(async move {
            let Ok(Ok((stream, _))) = tokio::time::timeout(Duration::from_secs(30), listener.accept()).await else {
                warn!("simulated phone: receiver never connected");
                return;
            };
            if let Err(e) = run_source(stream, cfg, ev_tx, cmd_rx).await {
                warn!("simulated phone RTSP ended: {e}");
            }
        });
        {
            let log = log.clone();
            tokio::spawn(async move {
                let mut port = 0u16;
                let mut mode = (1280u32, 720u32, 30u32);
                while let Some(ev) = ev_rx.recv().await {
                    match ev {
                        SourceEvent::Negotiated { video, audio, .. } => {
                            mode = (video.width, video.height, video.fps);
                            log(format!("simulated phone: chose {video}{}", audio.map(|a| format!(", {a}")).unwrap_or_default()));
                        }
                        SourceEvent::Setup { sink_rtp_port } => port = sink_rtp_port,
                        SourceEvent::Play => {
                            let c = TestSourceConfig {
                                port,
                                width: mode.0,
                                height: mode.1,
                                fps: mode.2,
                                caption: CAPTION.into(),
                                ..Default::default()
                            };
                            match TestSource::start(&c) {
                                Ok(s) => *media.lock().unwrap() = Some(s),
                                Err(e) => log(format!("simulated phone: cannot start test stream: {e}")),
                            }
                        }
                        SourceEvent::IdrRequested => {
                            if let Some(m) = media.lock().unwrap().as_ref() {
                                m.force_keyframe();
                            }
                        }
                        SourceEvent::Closed { reason } => {
                            info!(%reason, "simulated phone stopped");
                            media.lock().unwrap().take();
                            break;
                        }
                        _ => {}
                    }
                }
                media.lock().unwrap().take();
            });
        }
        Ok(Self { rtsp_addr, inputs, commands: cmd_tx })
    }

    /// Like the user pressing "disconnect" on the phone.
    pub fn stop(&self) {
        let _ = self.commands.send(SourceCommand::Teardown);
    }
}
