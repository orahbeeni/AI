//! Renders the window in representative states to PNG files, headless:
//! `WIFISHARE_SHOTS=dir cargo test gui::tests -- --ignored --test-threads=1`

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::app::{App, AppCtx, Tab};
use crate::config::{Config, Mode, Paths, Peers, Role};
use crate::engine::Engine;
use crate::geom::{Rect, Screen, Side};
use crate::proto::Os;
use crate::status::{Approval, AudioLink, ClientInfo, ClipEvent, Discovered, Hub, LinkState, PairPrompt, Rtt};

fn paths(name: &str) -> Paths {
    let dir = std::env::temp_dir().join(format!("wifishare-gui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Paths::new(Some(dir)).unwrap()
}

fn thinkpad() -> Screen {
    Screen { monitors: vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1920, 1080), Rect::new(3840, 0, 1920, 1080)] }
}

fn render(name: &str, paths: Paths, hub: Arc<Hub>, size: [f32; 2], dark: bool) {
    render_tab(name, paths, hub, size, dark, Tab::Overview);
}

fn render_tab(name: &str, paths: Paths, hub: Arc<Hub>, size: [f32; 2], dark: bool, tab: Tab) {
    let Some(dir) = std::env::var_os("WIFISHARE_SHOTS") else { return };
    let engine = Arc::new(Mutex::new(Engine::new(paths.clone(), hub.clone())));
    let ctx = AppCtx { engine, hub, paths, commands: None, has_tray: true, quit: Default::default(), start_minimized: false };
    let mut h = egui_kittest::Harness::builder().with_size(size).build_eframe(|cc| App::new(cc, ctx));
    h.ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
    h.state_mut().set_tab(tab);
    h.run_steps(4);
    let img = h.render().expect("render");
    let path = std::path::Path::new(&dir).join(format!("{name}.png"));
    img.save(&path).unwrap();
    println!("wrote {}", path.display());
}

fn pin(paths: &Paths, name: &str, role: Role, byte: u8) {
    let mut peers = Peers::load(paths).unwrap();
    peers.pin(name, &[byte; 32], role);
    peers.save(paths).unwrap();
}

#[test]
#[ignore]
fn screenshots() {
    // A Mac server driving the ThinkPad on its left, with a pairing request open.
    let p = paths("server");
    let mut cfg = Config { mode: Some(Mode::Server), name: "Omars-MacBook-Pro".into(), ..Config::default() };
    cfg.set_side("omar-ThinkPad-P50", Side::Left);
    cfg.save(&p).unwrap();
    pin(&p, "omar-ThinkPad-P50", Role::Client, 1);
    pin(&p, "office-pc", Role::Client, 2);
    let hub = Hub::new(Approval::Gui);
    let rtt = Rtt::default();
    rtt.set(4);
    hub.update(|s| {
        s.running = true;
        s.capturing = true;
        s.local_screen = Some(Screen::single(1512, 982));
        s.clients = vec![ClientInfo {
            conn: 1,
            name: "omar-ThinkPad-P50".into(),
            os: Os::Linux,
            addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 192)),
            screen: thinkpad(),
            side: Side::Left,
            since: Instant::now() - Duration::from_secs(600),
            rtt,
        }];
        s.active = Some("omar-ThinkPad-P50".into());
        s.audio = vec![AudioLink {
            peer: "omar-ThinkPad-P50".into(),
            sending: false,
            since: Instant::now() - Duration::from_secs(300),
            packets: Arc::new(9990.into()),
            lost: Arc::new(10.into()),
        }];
        s.clipboard = Some(ClipEvent { text: "Received 2 files (2.9 MB)".into(), at: Instant::now() - Duration::from_secs(12) });
    });
    let done = hub.transfer("Sending \"Screenshots\"".into(), 40 << 20);
    done.add(13 << 20);
    render("server", p.clone(), hub.clone(), [880.0, 900.0], false);
    render("server-dark", p.clone(), hub.clone(), [880.0, 760.0], true);
    let _answer = hub.ask_pair(PairPrompt {
        id: 0,
        peer: "office-pc".into(),
        os: Os::Windows,
        addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 40)),
        code: "482 913".into(),
        incoming: true,
    });
    render("server-pairing", p.clone(), hub.clone(), [880.0, 760.0], false);
    render_tab("settings", p, hub, [880.0, 1000.0], false, Tab::Settings);
    drop(done);

    // The ThinkPad looking for its Mac after it left Wi-Fi range.
    let p = paths("client");
    let mut cfg = Config { mode: Some(Mode::Client), name: "omar-ThinkPad-P50".into(), ..Config::default() };
    cfg.audio.send = true;
    cfg.save(&p).unwrap();
    pin(&p, "Omars-MacBook-Pro", Role::Server, 3);
    let hub = Hub::new(Approval::Gui);
    hub.update(|s| {
        s.running = true;
        s.local_screen = Some(thinkpad());
        s.link = LinkState::Searching {
            server: "Omars-MacBook-Pro".into(),
            since: Instant::now() - Duration::from_secs(47),
            error: Some("connecting to 192.168.1.25:24900 timed out".into()),
        };
        s.discovered = vec![Discovered {
            name: "studio-imac".into(),
            addr: SocketAddr::from(([192, 168, 1, 31], 24900)),
            key: vec![9; 32],
            seen: Instant::now(),
        }];
    });
    render("client-searching", p.clone(), hub.clone(), [880.0, 700.0], false);
    let rtt = Rtt::default();
    rtt.set(6);
    hub.update(|s| {
        s.link = LinkState::Connected {
            server: "Omars-MacBook-Pro".into(),
            addr: SocketAddr::from(([192, 168, 1, 25], 24900)),
            since: Instant::now() - Duration::from_secs(3600 * 2 + 600),
            rtt,
        };
        s.audio = vec![AudioLink {
            peer: "Omars-MacBook-Pro".into(),
            sending: true,
            since: Instant::now() - Duration::from_secs(1500),
            packets: Arc::default(),
            lost: Arc::default(),
        }];
    });
    render("client-connected", p, hub, [880.0, 820.0], true);

    // First run: nothing chosen yet.
    let p = paths("fresh");
    render("first-run", p, Hub::new(Approval::Gui), [880.0, 620.0], false);
}
