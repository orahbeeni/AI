//! Miracast receiver: shows a Samsung DeX / Android / Windows screen cast over
//! Wi-Fi Display in a resizable desktop window.

mod config;
mod engine;
mod share;
mod sim;
mod ui;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand, ValueEnum};
use tracing::info;

use crate::config::{Decoder, Settings};
use crate::engine::{EngineHandle, EngineOptions, Phase};

#[derive(Parser, Debug)]
#[command(name = "miracast-receiver", version, about = "Wireless display (Miracast) receiver window")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Name shown to phones and PCs (saved for next time).
    #[arg(long)]
    name: Option<String>,
    /// Start a simulated phone right away (no hardware needed).
    #[arg(long)]
    simulate: bool,
    /// Connect straight to a source's RTSP address instead of waiting.
    #[arg(long, value_name = "IP:PORT")]
    connect: Option<SocketAddr>,
    /// Preferred UDP port for the video stream.
    #[arg(long, default_value_t = 19000)]
    rtp_port: u16,
    /// Don't use Wi-Fi Direct.
    #[arg(long)]
    no_wifi_direct: bool,
    /// Don't use Miracast over Infrastructure (MS-MICE).
    #[arg(long)]
    no_infrastructure: bool,
    #[arg(long, value_enum)]
    decoder: Option<DecoderArg>,
    /// Start in full screen.
    #[arg(long)]
    fullscreen: bool,
    /// GUI self-test: simulate a phone, check picture and input, save a
    /// screenshot to this PNG and exit (status 0 = pass).
    #[arg(long, value_name = "PNG")]
    selftest: Option<PathBuf>,
    /// Save a screenshot after --shot-after seconds and exit (UI checks).
    #[arg(long, value_name = "PNG", hide = true)]
    shot: Option<PathBuf>,
    #[arg(long, default_value_t = 3.0, hide = true)]
    shot_after: f32,
    /// Open the settings window at start.
    #[arg(long, hide = true)]
    show_settings: bool,
    /// Show the stats overlay at start.
    #[arg(long, hide = true)]
    overlay: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum DecoderArg {
    Auto,
    Software,
    Hardware,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run a simulated phone (Wi-Fi Display source) that a receiver can connect to.
    SimSource {
        #[arg(long, default_value = "127.0.0.1:7236")]
        listen: SocketAddr,
        #[arg(long, default_value_t = 1280)]
        width: u32,
        #[arg(long, default_value_t = 720)]
        height: u32,
        #[arg(long, default_value_t = 30)]
        fps: u32,
        /// Stream with an external `gst-launch-1.0` process instead of in-process GStreamer.
        #[arg(long)]
        gst_launch: bool,
    },
    /// Headless end-to-end check (simulated phone -> RTSP -> decode -> input) without a window.
    Selftest {
        #[arg(long, default_value_t = 4)]
        seconds: u64,
    },
    /// Check this computer: decoders, Wi-Fi Direct permissions, mDNS.
    Doctor,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,zbus=warn".into()))
        .with_target(false)
        .init();
    let cli = Cli::parse();
    if let Err(e) = wfd_media::init() {
        eprintln!("GStreamer is not available: {e}");
        std::process::exit(3);
    }
    let code = match cli.command {
        Some(Command::SimSource { listen, width, height, fps, gst_launch }) => sim_source(listen, width, height, fps, gst_launch),
        Some(Command::Selftest { seconds }) => headless_selftest(seconds),
        Some(Command::Doctor) => doctor(),
        None => run_gui(cli),
    };
    std::process::exit(code);
}

fn settings_from(cli: &Cli) -> Settings {
    let mut s = Settings::load();
    if let Some(n) = &cli.name {
        s.name = n.clone();
        s.save();
    }
    if cli.no_wifi_direct {
        s.wifi_direct = false;
    }
    if cli.no_infrastructure {
        s.infrastructure = false;
    }
    if let Some(d) = cli.decoder {
        s.decoder = match d {
            DecoderArg::Auto => Decoder::Auto,
            DecoderArg::Software => Decoder::Software,
            DecoderArg::Hardware => Decoder::Hardware,
        };
    }
    s
}

fn run_gui(cli: Cli) -> i32 {
    let mut settings = settings_from(&cli);
    let selftest = cli.selftest.clone().map(|p| ui::SelfTest::new(Some(p)));
    let shot = cli.shot.clone().map(|p| ui::Shot::new(p, Duration::from_secs_f32(cli.shot_after)));
    let show_settings = cli.show_settings;
    if cli.overlay {
        settings.show_overlay = true;
    }
    if selftest.is_some() {
        // don't touch the user's network setup during a self-test
        settings.wifi_direct = false;
        settings.infrastructure = false;
        settings.auto_accept = true;
    }
    let opts = EngineOptions {
        settings: settings.clone(),
        connect: cli.connect,
        rtp_port: cli.rtp_port,
        simulate_on_start: cli.simulate || selftest.is_some(),
        audio_sink: selftest.as_ref().map(|_| "fakesink".to_string()),
    };
    let title = format!("{} — Miracast Receiver", settings.name);
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_app_id("miracast-receiver")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 320.0])
            .with_fullscreen(cli.fullscreen),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let result = eframe::run_native(
        "miracast-receiver",
        native,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            let engine = EngineHandle::start(opts, Arc::new(move || ctx.request_repaint()));
            let mut app = ui::ReceiverApp::new(cc, engine, settings, selftest);
            app.shot = shot;
            if show_settings {
                app.open_settings();
            }
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = result {
        eprintln!("cannot open the window: {e}");
        return 4;
    }
    ui::SELFTEST_EXIT.load(std::sync::atomic::Ordering::SeqCst)
}

fn headless_selftest(seconds: u64) -> i32 {
    let mut settings = Settings::load();
    settings.wifi_direct = false;
    settings.infrastructure = false;
    settings.auto_accept = true;
    let opts = EngineOptions { settings, connect: None, rtp_port: 0, simulate_on_start: true, audio_sink: Some("fakesink".into()) };
    let mut engine = EngineHandle::start(opts, Arc::new(|| {}));
    let start = Instant::now();
    let mut frames_seen = 0u64;
    let mut injected: Option<Instant> = None;
    let deadline = Duration::from_secs(30 + seconds);
    loop {
        std::thread::sleep(Duration::from_millis(100));
        while engine.frames.take().is_some() {
            frames_seen += 1;
        }
        let s = engine.snapshot();
        if start.elapsed() > deadline {
            eprintln!("FAIL: timed out in phase {:?}", s.session.phase);
            return 1;
        }
        let Some(m) = &s.media else { continue };
        if s.session.phase != Phase::Playing || m.frames < 30 {
            continue;
        }
        if injected.is_none()
            && let Some(tx) = &s.input
        {
            use wfd_core::uibc::{InputEvent, MouseButton};
            let _ = tx.send(InputEvent::PointerMove { x: 100, y: 100, dx: 10, dy: 5 });
            let _ = tx.send(InputEvent::Button { button: MouseButton::Right, pressed: true, x: 100, y: 100 });
            let _ = tx.send(InputEvent::Button { button: MouseButton::Right, pressed: false, x: 100, y: 100 });
            let _ = tx.send(InputEvent::Key { usage: 0x04, pressed: true, modifiers: 0 });
            let _ = tx.send(InputEvent::Key { usage: 0x04, pressed: false, modifiers: 0 });
            injected = Some(Instant::now());
        }
        // run for the requested time, and give injected input a moment to arrive
        let settled = injected.is_none_or(|t| t.elapsed() > Duration::from_secs(1));
        if start.elapsed() < Duration::from_secs(seconds) || !settled {
            continue;
        }
        let inputs = s.sim_inputs.as_ref().map(|i| i.lock().unwrap().clone()).unwrap_or_default();
        println!("session:  {} via {:?}", s.session.peer_name, s.session.kind);
        println!("stream:   {} · {}", s.session.video.as_deref().unwrap_or("-"), s.session.audio.as_deref().unwrap_or("-"));
        println!(
            "decoder:  {} · {}x{} · {:.1} fps · {:.0} kbit/s · lost {}",
            m.decoder, m.width, m.height, m.fps, m.bitrate_kbps, m.lost_packets
        );
        println!("frames:   {} decoded, {} handed to the renderer", m.frames, frames_seen);
        println!("input:    {}", s.session.uibc.as_ref().map_or("off".into(), |u| u.description.clone()));
        for i in &inputs {
            println!("  phone received: {i}");
        }
        let ok = m.fps > 20.0 && frames_seen > 20 && (s.input.is_none() || inputs.iter().any(|i| i.contains("keyboard")));
        engine.shutdown();
        println!("{}", if ok { "PASS" } else { "FAIL" });
        return if ok { 0 } else { 1 };
    }
}

fn sim_source(listen: SocketAddr, width: u32, height: u32, fps: u32, gst_launch: bool) -> i32 {
    use wfd_core::driver::run_source;
    use wfd_core::source::{SourceConfig, SourceEvent};
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async move {
        let listener = match tokio::net::TcpListener::bind(listen).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("cannot listen on {listen}: {e}");
                return 1;
            }
        };
        println!("Simulated phone: RTSP on {listen}. Connect with: miracast-receiver --connect {listen}");
        loop {
            let Ok((stream, peer)) = listener.accept().await else { continue };
            println!("receiver connected from {peer}");
            let cfg =
                SourceConfig { host: listen.ip().to_string(), max_width: width, max_height: height, max_fps: fps, ..Default::default() };
            let (etx, mut erx) = tokio::sync::mpsc::unbounded_channel();
            let (_ctx, crx) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(run_source(stream, cfg, etx, crx));
            let mut port = 0;
            let mut mode = (width, height, fps);
            let mut media: Option<wfd_media::TestSource> = None;
            let mut child: Option<std::process::Child> = None;
            while let Some(ev) = erx.recv().await {
                println!("  {ev:?}");
                match ev {
                    SourceEvent::Negotiated { video, .. } => mode = (video.width, video.height, video.fps),
                    SourceEvent::Setup { sink_rtp_port } => port = sink_rtp_port,
                    SourceEvent::Play => {
                        let c = wfd_media::TestSourceConfig {
                            host: peer.ip().to_string(),
                            port,
                            width: mode.0,
                            height: mode.1,
                            fps: mode.2,
                            ..Default::default()
                        };
                        if gst_launch {
                            let line = wfd_media::TestSource::launch_line(&c);
                            println!("  running: gst-launch-1.0 {line}");
                            child = std::process::Command::new("sh").arg("-c").arg(format!("exec gst-launch-1.0 -q {line}")).spawn().ok();
                        } else {
                            media = wfd_media::TestSource::start(&c).ok();
                        }
                    }
                    SourceEvent::IdrRequested => {
                        if let Some(m) = &media {
                            m.force_keyframe();
                        }
                    }
                    SourceEvent::Closed { .. } => break,
                    _ => {}
                }
            }
            drop(media);
            if let Some(mut c) = child {
                let _ = c.kill();
            }
            println!("session over; waiting for the next connection");
        }
    })
}

fn doctor() -> i32 {
    println!("Miracast receiver — system check\n");
    let decoders = wfd_media::available_decoders();
    println!(
        "GStreamer H.264 decoders: {}",
        if decoders.is_empty() { "NONE (install gstreamer1.0-libav)".into() } else { decoders.join(", ") }
    );
    match wfd_media::pick_decoder(wfd_media::DecoderChoice::Auto) {
        Ok((_, n)) => println!("  would use: {n}"),
        Err(e) => println!("  problem: {e}"),
    }
    println!(
        "Wi-Fi Direct: {}",
        match wfd_transport::wifi_direct_support() {
            Ok(()) => "supported on this OS".to_string(),
            Err(e) => e.to_string(),
        }
    );
    #[cfg(target_os = "linux")]
    {
        use wfd_transport::linux_p2p::{netlink, wpa::Wpa};
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async {
            match Wpa::connect().await {
                Ok(w) => match w.capabilities().await {
                    Ok(c) => {
                        println!(
                            "  wpa_supplicant D-Bus: OK (P2P {})",
                            if c.iter().any(|x| x == "p2p") { "supported" } else { "NOT supported" }
                        );
                        if let Ok(ifs) = w.interfaces().await {
                            for (_, n) in ifs {
                                println!("    interface {n}");
                            }
                        }
                    }
                    Err(e) if e.access_denied => println!("  wpa_supplicant D-Bus: ACCESS DENIED -> run scripts/setup-linux.sh"),
                    Err(e) => println!("  wpa_supplicant D-Bus: {e}"),
                },
                Err(e) => println!("  system D-Bus: {e}"),
            }
        });
        let missing = netlink::missing_caps();
        if missing.is_empty() {
            println!("  network capabilities: OK");
        } else {
            println!(
                "  network capabilities missing: {} -> run scripts/setup-linux.sh (setcap on the installed binary)",
                missing.join(", ")
            );
        }
    }
    match std::net::TcpListener::bind("0.0.0.0:7250") {
        Ok(_) => println!("MS-MICE TCP 7250: free"),
        Err(e) => println!("MS-MICE TCP 7250: {e}"),
    }
    println!("Primary IPv4: {}", wfd_transport::net::primary_ipv4().map_or("none".into(), |i| i.to_string()));
    info!("doctor finished");
    0
}
