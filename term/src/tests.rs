//! Tests for the parts that do not need a window: split trees, key encoding, settings and layouts.

use alacritty_terminal::term::TermMode;
use eframe::egui::{Key, Modifiers, Rect, pos2};

use crate::config::{Combo, Rgb, Settings, Theme};
use crate::input::{MouseAction, key_bytes, mouse_report, paste_bytes};
use crate::layout::{Dir, Nav, Node, neighbour};
use crate::pane::SavedCommand;
use crate::session::{LayoutFile, PaneSpec, TabSpec};

fn three_panes() -> Node<u64> {
    // 1 | (2 over 3)
    let mut n = Node::Leaf(1);
    n.split(&1, Dir::Row, 2);
    n.split(&2, Dir::Col, 3);
    n
}

#[test]
fn split_and_remove() {
    let mut n = three_panes();
    assert_eq!(n.leaves(), vec![1, 2, 3]);
    assert!(n.remove(&2));
    assert_eq!(n.leaves(), vec![1, 3]);
    assert!(n.remove(&1));
    assert_eq!(n.leaves(), vec![3]);
    assert!(!n.remove(&3), "the last terminal is removed by closing the tab");
}

#[test]
fn layout_and_neighbours() {
    let n = three_panes();
    let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 600.0));
    let (mut rects, mut divs) = (Vec::new(), Vec::new());
    n.layout(area, 4.0, &mut rects, &mut divs);
    assert_eq!(rects.len(), 3);
    assert_eq!(divs.len(), 2);
    let r1 = rects[0].1;
    assert!((r1.width() - 498.0).abs() < 1.0, "{r1:?}");
    assert_eq!(neighbour(&rects, &1, Nav::Right), Some(2));
    assert_eq!(neighbour(&rects, &2, Nav::Down), Some(3));
    assert_eq!(neighbour(&rects, &3, Nav::Up), Some(2));
    assert_eq!(neighbour(&rects, &3, Nav::Left), Some(1));
    assert_eq!(neighbour(&rects, &1, Nav::Left), None);
}

#[test]
fn nudge_and_equalize() {
    let mut n = three_panes();
    assert!(n.nudge(&3, Nav::Up, 0.1));
    assert!(n.nudge(&1, Nav::Right, 0.1));
    let Node::Split { ratio, b, .. } = &n else { panic!() };
    assert!((ratio - 0.6).abs() < 1e-5);
    let Node::Split { ratio: inner, .. } = b.as_ref() else { panic!() };
    assert!((inner - 0.4).abs() < 1e-5);

    // Three side by side end up a third each.
    let mut row = Node::Leaf(1);
    row.split(&1, Dir::Row, 2);
    row.split(&2, Dir::Row, 3);
    row.equalize();
    let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(900.0, 100.0));
    let (mut rects, mut divs) = (Vec::new(), Vec::new());
    row.layout(area, 0.0, &mut rects, &mut divs);
    for (_, r) in rects {
        assert!((r.width() - 300.0).abs() <= 1.0, "{r:?}");
    }
}

#[test]
fn key_encoding() {
    let none = Modifiers::NONE;
    let ctrl = Modifiers { ctrl: true, ..none };
    let alt = Modifiers { alt: true, ..none };
    let m = TermMode::empty();
    assert_eq!(key_bytes(Key::Enter, none, m, true), Some(b"\r".to_vec()));
    assert_eq!(key_bytes(Key::C, ctrl, m, true), Some(vec![3]));
    assert_eq!(key_bytes(Key::A, none, m, true), None, "plain text arrives as a Text event");
    assert_eq!(key_bytes(Key::B, alt, m, true), Some(b"\x1bb".to_vec()));
    assert_eq!(key_bytes(Key::B, alt, m, false), None);
    assert_eq!(key_bytes(Key::ArrowUp, none, m, true), Some(b"\x1b[A".to_vec()));
    assert_eq!(key_bytes(Key::ArrowUp, none, TermMode::APP_CURSOR, true), Some(b"\x1bOA".to_vec()));
    assert_eq!(key_bytes(Key::ArrowRight, ctrl, m, true), Some(b"\x1b[1;5C".to_vec()));
    assert_eq!(key_bytes(Key::Tab, Modifiers { shift: true, ..none }, m, true), Some(b"\x1b[Z".to_vec()));
    assert_eq!(key_bytes(Key::F5, none, m, true), Some(b"\x1b[15~".to_vec()));
    assert_eq!(key_bytes(Key::Backspace, none, m, true), Some(vec![0x7f]));
}

#[test]
fn paste_and_mouse() {
    assert_eq!(paste_bytes("a\nb", TermMode::empty()), b"a\rb".to_vec());
    assert_eq!(paste_bytes("x\x1b[201~y", TermMode::BRACKETED_PASTE), b"\x1b[200~xy\x1b[201~".to_vec());
    let sgr = TermMode::SGR_MOUSE;
    assert_eq!(mouse_report(0, MouseAction::Press, 4, 2, Modifiers::NONE, sgr), Some(b"\x1b[<0;5;3M".to_vec()));
    assert_eq!(mouse_report(0, MouseAction::Release, 4, 2, Modifiers::NONE, sgr), Some(b"\x1b[<0;5;3m".to_vec()));
    assert_eq!(mouse_report(0, MouseAction::Press, 0, 0, Modifiers::NONE, TermMode::empty()), Some(b"\x1b[M\x20\x21\x21".to_vec()));
}

#[test]
fn combos() {
    let c = Combo::parse("Ctrl+Shift+O").unwrap();
    assert!(c.matches(Key::O, Modifiers { ctrl: true, shift: true, ..Modifiers::NONE }));
    assert!(!c.matches(Key::O, Modifiers { ctrl: true, ..Modifiers::NONE }));
    assert!(Combo::parse("Ctrl+Plus").unwrap().matches(Key::Equals, Modifiers { ctrl: true, ..Modifiers::NONE }));
    assert!(Combo::parse("Ctrl+Nope").is_none());
    assert!(Combo::parse("Alt+PageUp").is_some());
    // Every default binding parses.
    let s = Settings::default();
    for (action, keys) in &s.keybindings {
        for k in keys {
            assert!(Combo::parse(k).is_some(), "{action}: {k}");
        }
    }
}

#[test]
fn settings_and_layout_round_trip() {
    let mut s = Settings { theme: Theme::presets().remove(3), ..Default::default() };
    s.theme.background = Rgb(1, 2, 3);
    let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back, s);
    // Older files with missing fields still load.
    let partial: Settings = serde_json::from_str(r#"{"font_size": 15.0}"#).unwrap();
    assert_eq!(partial.font_size, 15.0);

    let spec = |cwd: &str, cmds: &[(&str, bool)]| PaneSpec {
        cwd: Some(cwd.into()),
        name: None,
        commands: cmds.iter().map(|(c, a)| SavedCommand { command: c.to_string(), auto_run: *a, ..Default::default() }).collect(),
        ..Default::default()
    };
    let mut root = Node::Leaf(spec("/opt/hybris/bin/platform", &[(". ./setantenv.sh", true), ("./hybrisserver.sh debug", true)]));
    if let Node::Leaf(first) = root.clone() {
        root = Node::Split { dir: Dir::Row, ratio: 0.6, a: Box::new(Node::Leaf(first)), b: Box::new(Node::Leaf(spec("/tmp", &[]))) };
    }
    let l = LayoutFile {
        name: "hybris".into(),
        tabs: vec![TabSpec { title: Some("server".into()), root, focused: 1, layout: None }],
        ..Default::default()
    };
    let back: LayoutFile = serde_json::from_str(&serde_json::to_string_pretty(&l).unwrap()).unwrap();
    assert_eq!(back.tabs[0].root.leaves(), l.tabs[0].root.leaves());
    assert_eq!(back.tabs[0].focused, 1);
}

#[test]
fn real_shell_round_trip() {
    use std::time::{Duration, Instant};

    use crate::pane::{Pane, TermSize, all_text};

    let ctx = eframe::egui::Context::default();
    let mut s = Settings::default();
    if cfg!(unix) {
        s.shell = "/bin/sh".into();
    }
    let dir = std::env::temp_dir().canonicalize().unwrap();
    let mut p = Pane::spawn(1, &s, Some(dir.clone()), TermSize { cols: 80, rows: 24 }, (8.0, 16.0), &ctx, None).unwrap();
    p.run_command("echo hello-$((40+2))");
    p.run_command("cd / && pwd");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text = all_text(&p.term.lock());
        if text.contains("hello-42") && text.lines().any(|l| l.trim().trim_start_matches("$ ") == "/") {
            break;
        }
        assert!(Instant::now() < deadline, "no output from the shell:\n{text}");
        std::thread::sleep(Duration::from_millis(20));
    }
    p.resize(TermSize { cols: 100, rows: 30 }, (8.0, 16.0));
    assert_eq!(alacritty_terminal::grid::Dimensions::columns(&*p.term.lock()), 100);
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        assert_eq!(p.current_dir(), std::path::PathBuf::from("/"));
    }
    p.run_command("exit");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !p.exited() {
        assert!(Instant::now() < deadline, "shell did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Renders the whole app offscreen with real shells and writes PNGs to `TERM_SHOTS` (a folder).
/// Run with: TERM_SHOTS=/some/dir cargo test visual -- --ignored --nocapture
#[test]
#[ignore]
fn visual() {
    use std::time::Duration;

    let Ok(dir) = std::env::var("TERM_SHOTS") else { return };
    // A two-tab layout: a server pane with auto-run commands beside two plain ones.
    let pane = |cwd: &str, name: Option<&str>, cmds: &[(&str, bool)]| PaneSpec {
        cwd: Some(cwd.into()),
        name: name.map(String::from),
        commands: cmds.iter().map(|(c, a)| SavedCommand { command: c.to_string(), auto_run: *a, ..Default::default() }).collect(),
        ..Default::default()
    };
    let root = Node::Split {
        dir: Dir::Row,
        ratio: 0.65,
        a: Box::new(Node::Leaf(pane(
            "/usr/bin",
            Some("hybris server"),
            &[
                ("echo starting in $PWD", true),
                ("for i in 1 2 3; do echo \"INFO [main] boot step $i\"; done", true),
                ("echo not auto", false),
            ],
        ))),
        b: Box::new(Node::Split {
            dir: Dir::Col,
            ratio: 0.5,
            a: Box::new(Node::Leaf(pane("/tmp", None, &[]))),
            b: Box::new(Node::Leaf(pane("/etc", None, &[("pwd", true)]))),
        }),
    };
    let l = LayoutFile {
        name: "demo".into(),
        tabs: vec![
            TabSpec { title: None, root, focused: 0, layout: None },
            TabSpec { title: None, root: Node::Leaf(pane("/", None, &[])), focused: 0, layout: None },
        ],
        ..Default::default()
    };
    let layout_path = format!("{dir}/demo-layout.json");
    let import_path = format!("{dir}/term-settings.json");
    std::fs::write(&layout_path, serde_json::to_string_pretty(&l).unwrap()).unwrap();

    // A private config folder with a few saved layouts, so the user's own settings are untouched.
    let cfg = format!("{dir}/config");
    let _ = std::fs::remove_dir_all(&cfg);
    // SAFETY: set before any terminal or thread that reads the environment is started.
    unsafe { std::env::set_var("TERM_CONFIG_DIR", &cfg) };
    let s = Settings { theme: Theme::presets().into_iter().find(|t| t.name == "Nord").unwrap(), ..Default::default() };
    s.save();
    for (name, tabs) in [("hybris", 2usize), ("cvv2 debugging", 1), ("rust projects", 3)] {
        let mut l = l.clone();
        l.name = name.into();
        l.tabs.truncate(tabs);
        while l.tabs.len() < tabs {
            l.tabs.push(l.tabs[0].clone());
        }
        crate::session::save_layout(&l).unwrap();
    }
    crate::transfer::export(std::path::Path::new(&import_path), &Settings::default()).unwrap();
    let scenarios: &[(&str, &[&str])] = &[
        (
            "splits",
            &[
                "run:ls -G /",
                "run:printf '\\033[31mred \\033[1;32mbold green\\033[0m \\033[44m blue bg \\033[0m \\033[4munder\\033[0m \\033[3mitalic\\033[0m 日本語 ✓ ─│┼ \\033[38;5;208m256-orange\\033[0m \\033[38;2;100;200;255mtruecolor\\033[0m\\n'",
                "split_vertical",
                "run:for i in $(seq 1 400); do echo \"INFO  [hybris] cvv2 log line $i\"; sleep 0.01; done",
                "select:5",
                "split_horizontal",
                "run:echo bottom pane; vim --version | head -3",
            ],
        ),
        ("prefs", &["preferences"]),
        ("launcher", &[]),
        ("import", &["import_file:IMPORT"]),
        ("menu", &["addcmd:git log --oneline -7", "addcmd:git rebase -i HEAD~7!", "addcmd:git push --force", "menu_preview"]),
        (
            "graphite",
            &[
                "set_theme:Graphite",
                "run:printf '\\033[1m> Task :ystartDebug\\033[0m\\nRunning hybrisPlatform on Tomcat...\\n\\033[1;32mBUILD SUCCESSFUL\\033[0m in 4s\\n\\033[31mred \\033[32mgreen \\033[33myellow \\033[34mblue \\033[35mmagenta \\033[36mcyan\\033[0m\\n'",
                "select:2",
                "new_tab",
                "new_tab",
                "prev_tab",
                "prev_tab",
            ],
        ),
        (
            "insert",
            &["addcmd:./start-jbr.sh!", "addcmd:./gradlew yall", "run:echo ran-ok", "insert:echo should-not-run-yet", "split_vertical"],
        ),
        (
            "commands",
            &[
                "run:cd ~/work/cloud_/core-customize 2>/dev/null; clear",
                "addcmd:clear!",
                "addcmd:./start-jbr.sh!",
                "addcmd:./gradlew yall",
                "commands",
            ],
        ),
        (
            "commands_light",
            &["set_theme:Solarized Light", "addcmd:clear!", "addcmd:./start-jbr.sh!", "commands", "split_vertical", "layouts"],
        ),
        (
            "tabs",
            &[
                "run:cd ~/Documents",
                "split_vertical",
                "new_tab",
                "run:cd /usr/local/share/a-rather-long-folder-name-here 2>/dev/null || cd /usr/local",
                "new_tab",
                "run:cd /tmp",
                "new_tab",
                "prev_tab",
            ],
        ),
        ("tabs_nord", &["set_theme:Nord", "run:cd ~/Documents", "split_vertical", "new_tab", "run:cd /tmp", "new_tab", "prev_tab"]),
        (
            "preview",
            &[
                "run:ls -G /",
                "split_vertical",
                "run:printf '\\033[31mred \\033[32mgreen \\033[33myellow \\033[34mblue \\033[35mmagenta \\033[36mcyan\\033[0m\\n'",
                "preferences",
                "theme:Dracula",
            ],
        ),
        ("layout", &["layout:LAYOUT", "new_tab", "next_tab"]),
        ("about", &["about"]),
        ("workflow", &["prefs_page:Workflow"]),
        ("goto", &["run:cd /etc", "split_vertical", "new_tab", "alert", "goto"]),
        ("searches", &["addsearch:ERROR!", "addsearch:Exception in thread", "addsearch:Started Application", "edit_searches"]),
        (
            "marks",
            &[
                "run:. $TERM_INTEGRATION_DIR/term.bash; clear",
                "run:ls /",
                "run:seq 1 5",
                "run:false",
                "record",
                "alert",
                "toast:cvv2 server: finished|“./hybrisserver.sh debug” took 4m 12s",
            ],
        ),
    ];
    let only = std::env::var("TERM_SHOT").ok();
    for (name, steps) in scenarios {
        if only.as_deref().is_some_and(|o| o != *name) {
            continue;
        }
        let mut h = egui_kittest::Harness::builder().with_size([1100.0, 640.0]).build_eframe(|cc| {
            crate::app::App::new(
                cc,
                crate::Args { layout: None, run: false, fresh: true, dir: (*name != "launcher").then(|| "/Users".into()) },
            )
        });
        h.run_steps(3);
        std::thread::sleep(Duration::from_millis(1200));
        for step in *steps {
            h.state_mut().script_step(&step.replace("LAYOUT", &layout_path).replace("IMPORT", &import_path));
            for _ in 0..6 {
                h.step();
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        for _ in 0..20 {
            h.step();
            std::thread::sleep(Duration::from_millis(100));
        }
        let img = h.render().expect("render");
        let path = format!("{dir}/{name}.png");
        img.save(&path).unwrap();
        println!("wrote {path}");
    }
}

/// Tests that point TERM_CONFIG_DIR somewhere take turns, as it is process-wide.
static CONFIG_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn layout_list_and_rename() {
    use crate::session;
    let _guard = CONFIG_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("term-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // SAFETY: no other test reads TERM_CONFIG_DIR (the visual test is run on its own).
    unsafe { std::env::set_var("TERM_CONFIG_DIR", &dir) };
    let pane = |cwd: &str, auto: bool| PaneSpec {
        cwd: Some(cwd.into()),
        name: None,
        commands: vec![SavedCommand { command: "ls".into(), auto_run: auto, ..Default::default() }],
        ..Default::default()
    };
    let root =
        Node::Split { dir: Dir::Row, ratio: 0.5, a: Box::new(Node::Leaf(pane("/a", true))), b: Box::new(Node::Leaf(pane("/a", false))) };
    let l = LayoutFile {
        name: "Hybris: dev".into(),
        tabs: vec![TabSpec { title: None, root, focused: 0, layout: None }],
        ..Default::default()
    };
    session::save_layout(&l).unwrap();
    let s = session::summaries();
    assert_eq!(s.len(), 1);
    assert_eq!((s[0].name.as_str(), s[0].tabs, s[0].terminals, s[0].auto_run), ("Hybris: dev", 1, 2, 1));
    assert_eq!(s[0].folders, vec!["/a".to_string()]);

    session::rename_layout("Hybris: dev", "hybris").unwrap();
    assert_eq!(session::list_layouts(), vec!["hybris".to_string()]);
    assert!(session::load_layout("hybris").is_some());
    session::save_layout(&LayoutFile { name: "other".into(), ..Default::default() }).unwrap();
    assert!(session::rename_layout("hybris", "other").is_err(), "renaming onto an existing layout must fail");
    let _ = std::fs::remove_dir_all(&dir);
    unsafe { std::env::remove_var("TERM_CONFIG_DIR") };
}

#[test]
fn app_icon_is_transparent_outside_the_border() {
    let icon = crate::icon();
    assert_eq!((icon.width, icon.height), (1024, 1024));
    let alpha = |x: usize, y: usize| icon.rgba[(y * 1024 + x) * 4 + 3];
    // Margins and the cut-off corners are fully transparent; the face is opaque.
    for (x, y) in [(0, 0), (1023, 0), (0, 1023), (1023, 1023), (80, 80), (943, 943), (512, 40)] {
        assert_eq!(alpha(x, y), 0, "pixel ({x}, {y}) should be transparent");
    }
    for (x, y) in [(512, 512), (200, 512), (512, 200), (512, 80)] {
        assert_eq!(alpha(x, y), 255, "pixel ({x}, {y}) should be opaque");
    }
}

/// Dragging a tab across others moves it once per tab crossed and never flips back and forth.
#[test]
#[ignore] // opens real shells and renders offscreen; run with --ignored
fn tab_drag_is_stable() {
    use eframe::egui::{Event, PointerButton, pos2};

    let mut h = egui_kittest::Harness::builder()
        .with_size([1100.0, 400.0])
        .build_eframe(|cc| crate::app::App::new(cc, crate::Args { layout: None, run: false, fresh: true, dir: Some("/tmp".into()) }));
    h.run_steps(2);
    h.state_mut().script_step("new_tab");
    h.state_mut().script_step("new_tab");
    h.run_steps(3);
    let start = h.state().tab_uids();
    assert_eq!(start.len(), 3);

    // Tabs start at x≈6 and are equally wide; grab the first one and drag it to the right.
    let press = |h: &mut egui_kittest::Harness<'_, crate::app::App>, x: f32, down: bool| {
        h.input_mut().events.push(Event::PointerMoved(pos2(x, 16.0)));
        h.input_mut().events.push(Event::PointerButton {
            pos: pos2(x, 16.0),
            button: PointerButton::Primary,
            pressed: down,
            modifiers: Default::default(),
        });
    };
    press(&mut h, 40.0, true);
    h.step();
    let mut orders = vec![h.state().tab_uids()];
    for step in 1..=60 {
        let x = 40.0 + step as f32 * 10.0;
        h.input_mut().events.push(Event::PointerMoved(pos2(x, 16.0)));
        h.step();
        orders.push(h.state().tab_uids());
    }
    press(&mut h, 640.0, false);
    h.step();
    let end = h.state().tab_uids();

    // The dragged tab only ever moves right: its index never decreases from one frame to the next.
    let pos_of = |o: &Vec<u64>| o.iter().position(|u| *u == start[0]).unwrap();
    for w in orders.windows(2) {
        assert!(pos_of(&w[1]) >= pos_of(&w[0]), "tab jumped back: {:?} -> {:?}", w[0], w[1]);
    }
    assert_eq!(end, vec![start[1], start[2], start[0]], "dragged tab should end up last");
}

/// Deleting a layout from the list: hover a row (not the highlighted one), click its 🗑, confirm.
#[test]
#[ignore] // opens a real shell and renders offscreen; run with --ignored
fn delete_layout_from_list() {
    use eframe::egui::{Event, PointerButton, pos2};
    use egui_kittest::kittest::Queryable;

    let cfg = std::env::temp_dir().join(format!("term-del-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cfg);
    // SAFETY: set before the app or any thread reads the environment.
    unsafe { std::env::set_var("TERM_CONFIG_DIR", &cfg) };
    for name in ["alpha", "beta", "gamma"] {
        let tab = TabSpec { title: None, root: Node::Leaf(PaneSpec::default()), focused: 0, layout: None };
        crate::session::save_layout(&LayoutFile { name: name.into(), tabs: vec![tab], ..Default::default() }).unwrap();
    }

    let mut h = egui_kittest::Harness::builder()
        .with_size([1100.0, 640.0])
        .build_eframe(|cc| crate::app::App::new(cc, crate::Args { layout: None, run: false, fresh: true, dir: None }));
    h.run_steps(4);
    assert!(h.query_by_label("Open a layout").is_some(), "the layout list opens at startup");

    // Rows are 62 pt tall with 6 pt gaps; the second row ("beta") sits 68 pt below the first.
    let first = h.get_by_label_contains("alpha").rect();
    let row_y = first.center().y + 68.0;
    let trash_x = 786.0;
    h.input_mut().events.push(Event::PointerMoved(pos2(500.0, row_y)));
    h.run_steps(2);
    h.input_mut().events.push(Event::PointerMoved(pos2(trash_x, row_y)));
    h.run_steps(2);
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton {
            pos: pos2(trash_x, row_y),
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        h.step();
    }
    h.run_steps(2);
    assert!(h.query_by_label("Delete layout?").is_some(), "clicking 🗑 on a hovered row asks for confirmation");
    h.get_by_label("Delete").click();
    h.run_steps(3);

    let left = crate::session::list_layouts();
    let _ = std::fs::remove_dir_all(&cfg);
    unsafe { std::env::remove_var("TERM_CONFIG_DIR") };
    assert_eq!(left, vec!["alpha".to_string(), "gamma".to_string()], "beta should be gone");
}

/// The saved-commands window always has an empty row: typing a command and pressing Enter keeps it
/// and leaves a fresh empty row focused for the next one.
#[test]
#[ignore] // opens a real shell and renders offscreen; run with --ignored
fn saved_commands_empty_row() {
    use eframe::egui::{Event, Key};

    let mut h = egui_kittest::Harness::builder()
        .with_size([1100.0, 640.0])
        .build_eframe(|cc| crate::app::App::new(cc, crate::Args { layout: None, run: false, fresh: true, dir: Some("/tmp".into()) }));
    h.run_steps(2);
    h.state_mut().script_step("commands");
    h.run_steps(3);
    let type_line = |h: &mut egui_kittest::Harness<'_, crate::app::App>, text: &str| {
        h.input_mut().events.push(Event::Text(text.into()));
        h.step();
        h.input_mut().events.push(Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        });
        h.input_mut().events.push(Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Default::default(),
        });
        h.run_steps(3);
    };
    type_line(&mut h, "./gradlew yall");
    assert_eq!(h.state().focused_commands(), vec![("./gradlew yall".to_string(), false)]);
    // Focus stayed in the new empty row: the next command goes straight in.
    type_line(&mut h, "./start-jbr.sh");
    assert_eq!(h.state().focused_commands().len(), 2);
    assert_eq!(h.state().focused_commands()[1].0, "./start-jbr.sh");
    if let Ok(dir) = std::env::var("TERM_SHOTS") {
        h.render().unwrap().save(format!("{dir}/commands_draft.png")).unwrap();
    }
}

/// "Save layout" defaults to the current tab only and can replace an existing layout with one click.
#[test]
#[ignore] // opens real shells and renders offscreen; run with --ignored
fn save_layout_replaces_existing() {
    use egui_kittest::kittest::Queryable;

    let cfg = std::env::temp_dir().join(format!("term-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cfg);
    // SAFETY: set before the app or any thread reads the environment.
    unsafe { std::env::set_var("TERM_CONFIG_DIR", &cfg) };
    for name in ["alpha", "beta"] {
        let tab = |cwd: &str| TabSpec {
            title: None,
            root: Node::Leaf(PaneSpec { cwd: Some(cwd.into()), ..Default::default() }),
            focused: 0,
            layout: None,
        };
        crate::session::save_layout(&LayoutFile { name: name.into(), tabs: vec![tab("/a"), tab("/b"), tab("/c")], ..Default::default() })
            .unwrap();
    }

    let mut h = egui_kittest::Harness::builder()
        .with_size([1100.0, 640.0])
        .build_eframe(|cc| crate::app::App::new(cc, crate::Args { layout: None, run: false, fresh: true, dir: Some("/tmp".into()) }));
    h.run_steps(2);
    h.state_mut().script_step("new_tab");
    h.state_mut().script_step("save_layout");
    h.run_steps(3);
    assert_eq!(h.state().save_dialog_only_tab(), Some(true), "current tab only by default");
    h.get_by_label("beta").click();
    h.run_steps(2);
    if let Ok(dir) = std::env::var("TERM_SHOTS") {
        h.render().unwrap().save(format!("{dir}/save_layout.png")).unwrap();
    }
    h.get_by_label("Replace").click();
    h.run_steps(3);

    let beta = crate::session::load_layout("beta").unwrap();
    let alpha = crate::session::load_layout("alpha").unwrap();
    let _ = std::fs::remove_dir_all(&cfg);
    unsafe { std::env::remove_var("TERM_CONFIG_DIR") };
    assert_eq!(beta.tabs.len(), 1, "beta now holds just the current tab");
    assert_eq!(alpha.tabs.len(), 3, "other layouts are untouched");
}

#[test]
fn export_and_import_round_trip() {
    use crate::{session, transfer};
    let _guard = CONFIG_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("term-xfer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // SAFETY: tests touching TERM_CONFIG_DIR hold CONFIG_DIR_LOCK.
    unsafe { std::env::set_var("TERM_CONFIG_DIR", dir.join("cfg")) };

    let mut s = Settings { font_size: 17.0, padding: 12.0, theme: Theme::presets().remove(4), ..Default::default() };
    s.keybindings.insert("new_tab".into(), vec!["Ctrl+Alt+T".into()]);
    let tab = |cwd: &str| TabSpec {
        title: None,
        root: Node::Leaf(PaneSpec {
            cwd: Some(cwd.into()),
            name: None,
            commands: vec![SavedCommand { command: "./start-jbr.sh".into(), auto_run: true, ..Default::default() }],
            ..Default::default()
        }),
        focused: 0,
        layout: None,
    };
    session::save_layout(&LayoutFile { name: "hybris".into(), tabs: vec![tab("/a")], ..Default::default() }).unwrap();
    session::save_layout(&LayoutFile { name: "git".into(), tabs: vec![tab("/b"), tab("/c")], ..Default::default() }).unwrap();
    let file = dir.join("term-settings.json");
    assert_eq!(transfer::export(&file, &s).unwrap(), 2);

    // A fresh machine: no layouts. Importing brings back settings and both layouts.
    std::fs::remove_dir_all(dir.join("cfg")).unwrap();
    let b = transfer::read(&file).unwrap();
    let got = b.settings.clone().unwrap();
    assert_eq!((got.font_size, got.padding, got.theme.name.as_str()), (17.0, 12.0, s.theme.name.as_str()));
    assert_eq!(got.keybindings["new_tab"], vec!["Ctrl+Alt+T".to_string()]);
    assert_eq!(transfer::import_layouts(&b, true).unwrap(), (2, 0));
    assert_eq!(session::list_layouts(), vec!["git".to_string(), "hybris".to_string()]);
    assert_eq!(session::load_layout("git").unwrap().tabs.len(), 2);
    assert!(session::load_layout("hybris").unwrap().tabs[0].root.leaves()[0].commands[0].auto_run);

    // Clashing names are skipped when asked to keep the local ones.
    assert_eq!(transfer::import_layouts(&b, false).unwrap(), (0, 2));
    // Anything else is refused.
    std::fs::write(dir.join("other.json"), r#"{"name": "x"}"#).unwrap();
    assert!(transfer::read(&dir.join("other.json")).is_err());

    let _ = std::fs::remove_dir_all(&dir);
    unsafe { std::env::remove_var("TERM_CONFIG_DIR") };
}

/// Closing the window or quitting asks first; Esc cancels, Enter (or the quit key again) quits.
#[test]
#[ignore] // opens a real shell and renders offscreen; run with --ignored
fn quit_asks_first() {
    use eframe::egui::{Key, ViewportEvent, ViewportId};

    let _guard = CONFIG_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = std::env::temp_dir().join(format!("term-quit-{}", std::process::id()));
    // SAFETY: tests touching TERM_CONFIG_DIR hold CONFIG_DIR_LOCK.
    unsafe { std::env::set_var("TERM_CONFIG_DIR", &cfg) };
    let mut h = egui_kittest::Harness::builder()
        .with_size([1100.0, 640.0])
        .build_eframe(|cc| crate::app::App::new(cc, crate::Args { layout: None, run: false, fresh: true, dir: Some("/tmp".into()) }));
    h.run_steps(2);
    let key = |h: &mut egui_kittest::Harness<'_, crate::app::App>, key: Key| {
        h.state_mut().script_step(&format!("key:{}", key.name()));
        h.run_steps(2);
    };

    // The window's close button.
    h.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.push(ViewportEvent::Close);
    h.step();
    // The harness keeps viewport events between frames; a real close request arrives once.
    h.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.clear();
    h.run_steps(2);
    assert_eq!(h.state().quit_state(), (true, false), "closing the window asks first and stays open");
    if let Ok(dir) = std::env::var("TERM_SHOTS") {
        h.render().unwrap().save(format!("{dir}/quit.png")).unwrap();
    }
    key(&mut h, Key::Escape);
    assert_eq!(h.state().quit_state(), (false, false), "Esc cancels");

    // Closing the last tab or terminal asks too, and Enter confirms.
    h.state_mut().script_step("close_tab");
    h.run_steps(1);
    assert_eq!(h.state().quit_state(), (true, false));
    key(&mut h, Key::Enter);
    assert!(h.state().quit_state().1, "Enter quits");

    let _ = std::fs::remove_dir_all(&cfg);
    unsafe { std::env::remove_var("TERM_CONFIG_DIR") };
}

/// A real bash with the shell integration: its prompts and commands are marked, the exit codes and
/// command text are captured, and the marks stay on the right lines while the output scrolls,
/// also once the scrollback is full and the oldest lines drop off the top.
#[test]
#[cfg(unix)]
fn bash_integration_marks_commands() {
    use std::time::{Duration, Instant};

    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line, Point};

    use crate::pane::{Pane, TermSize};

    if !std::path::Path::new("/bin/bash").exists() {
        return;
    }
    let _guard = CONFIG_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = std::env::temp_dir().join(format!("term-bash-{}", std::process::id()));
    // SAFETY: tests touching TERM_CONFIG_DIR hold CONFIG_DIR_LOCK.
    unsafe { std::env::set_var("TERM_CONFIG_DIR", &cfg) };
    crate::integration::ensure().unwrap();
    let rc = cfg.join("bashrc");
    std::fs::write(&rc, format!("PS1='$ '\n. '{}/term.bash'\n", crate::integration::dir())).unwrap();
    let ctx = eframe::egui::Context::default();

    let start = |scrollback: usize| {
        let mut s = Settings::default();
        s.shell = "/bin/bash".into();
        s.shell_args = vec!["--noprofile".into(), "--rcfile".into(), rc.display().to_string(), "-i".into()];
        s.scrollback_lines = scrollback;
        Pane::spawn(1, &s, Some(cfg.clone()), TermSize { cols: 80, rows: 24 }, (8.0, 16.0), &ctx, None).unwrap()
    };
    // Wait until the command called `last` has finished.
    let wait_for = |p: &Pane, last: &str| {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if p.shared.marks.lock().unwrap().blocks.iter().any(|b| b.command == last && b.end.is_some()) {
                break;
            }
            let marked: Vec<_> = p.shared.marks.lock().unwrap().blocks.iter().map(|b| (b.command.clone(), b.exit)).collect();
            assert!(Instant::now() < deadline, "{last:?} was never marked as finished: {marked:?}\n{}", p.scrollback_text(50));
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let line_text = |p: &Pane, key: i64| {
        let t = p.term.lock();
        let line = Line(p.shared.marks.lock().unwrap().line(key));
        t.bounds_to_string(Point::new(line, Column(0)), Point::new(line, t.last_column())).trim_end().to_string()
    };

    // Plenty of scrollback: everything stays.
    let p = start(10_000);
    p.run_command("echo hi");
    p.run_command("seq 1 100");
    p.run_command("false");
    wait_for(&p, "false");
    let blocks: Vec<_> = p.shared.marks.lock().unwrap().blocks.iter().filter(|b| b.end.is_some()).cloned().collect();
    assert_eq!((blocks[0].command.as_str(), blocks[0].exit), ("echo hi", Some(0)));
    assert_eq!(line_text(&p, blocks[0].output.unwrap()), "hi");
    assert_eq!(blocks[1].command, "seq 1 100");
    assert_eq!(line_text(&p, blocks[1].output.unwrap()), "1", "output start moved while 100 lines scrolled by");
    assert_eq!(line_text(&p, blocks[1].last_output_line().unwrap()), "100");
    assert_eq!((blocks[2].command.as_str(), blocks[2].exit), ("false", Some(1)));
    assert!(blocks[1].took.is_some());
    assert!(p.term.lock().grid().history_size() > 50, "the output really did scroll");

    // Narrowing the terminal reflows the text: the marks are dropped rather than left pointing at the wrong lines.
    let mut p = p;
    p.resize(TermSize { cols: 40, rows: 24 }, (8.0, 16.0));
    assert!(p.shared.marks.lock().unwrap().blocks.is_empty());
    drop(p);

    // A small scrollback that is full long before the end: lines drop off the top all the time.
    let p = start(40);
    p.run_command("seq 1 300");
    p.run_command("echo marker-one");
    p.run_command("seq 1 150");
    p.run_command("echo marker-two");
    wait_for(&p, "echo marker-two");
    let marks = p.shared.marks.lock().unwrap();
    assert!(marks.blocks.iter().all(|b| b.command != "seq 1 300"), "the first command scrolled out of the history");
    drop(marks);
    let last = p.shared.marks.lock().unwrap().blocks.iter().rev().find(|b| b.command == "echo marker-two").cloned().unwrap();
    assert_eq!(line_text(&p, last.output.unwrap()), "marker-two");
    assert_eq!(line_text(&p, last.prompt), "$ echo marker-two");
    assert!(p.term.lock().grid().history_size() == 40, "history is full");
    drop(p);

    // The user's own selection rides along with the text, and the marks are still right.
    let p = start(10_000);
    p.run_command("echo sel-target");
    wait_for(&p, "echo sel-target");
    {
        use alacritty_terminal::index::Side;
        use alacritty_terminal::selection::{Selection, SelectionType};
        let key = p.shared.marks.lock().unwrap().blocks.iter().find(|b| b.command == "echo sel-target").unwrap().output.unwrap();
        let line = Line(p.shared.marks.lock().unwrap().line(key));
        let mut t = p.term.lock();
        let mut sel = Selection::new(SelectionType::Simple, Point::new(line, Column(0)), Side::Left);
        sel.update(Point::new(line, Column(9)), Side::Right);
        t.selection = Some(sel);
        assert_eq!(t.selection_to_string().as_deref(), Some("sel-target"));
    }
    p.run_command("seq 1 150");
    wait_for(&p, "seq 1 150");
    assert_eq!(p.term.lock().selection_to_string().as_deref(), Some("sel-target"), "the selection moved with its text");
    let seq = p.shared.marks.lock().unwrap().blocks.iter().find(|b| b.command == "seq 1 150").cloned().unwrap();
    assert_eq!(line_text(&p, seq.output.unwrap()), "1");
    assert_eq!(line_text(&p, seq.last_output_line().unwrap()), "150");
    drop(p);

    unsafe { std::env::remove_var("TERM_CONFIG_DIR") };
    let _ = std::fs::remove_dir_all(&cfg);
}

/// Saved searches that alert, notification requests from programs, recording and scrollback restore.
#[test]
#[cfg(unix)]
fn watches_recording_and_scrollback() {
    use std::time::{Duration, Instant};

    use crate::pane::{Pane, PaneEvent, SavedSearch, TermSize, all_text};

    let _guard = CONFIG_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = std::env::temp_dir().join(format!("term-watch-{}", std::process::id()));
    // SAFETY: tests touching TERM_CONFIG_DIR hold CONFIG_DIR_LOCK.
    unsafe { std::env::set_var("TERM_CONFIG_DIR", &cfg) };
    let ctx = eframe::egui::Context::default();
    let mut s = Settings::default();
    s.shell = "/bin/sh".into();
    let size = TermSize { cols: 80, rows: 24 };
    let mut p = Pane::spawn(1, &s, Some(cfg.clone()), size, (8.0, 16.0), &ctx, None).unwrap();
    std::fs::create_dir_all(&cfg).unwrap();
    p.searches = vec![
        SavedSearch { pattern: "boom".into(), regex: false, alert: true },
        SavedSearch { pattern: r"^FAIL \d+$".into(), regex: true, alert: true },
        SavedSearch { pattern: "silent".into(), regex: false, alert: false },
    ];
    p.sync_watches();
    p.set_recording(true, &cfg.join("logs")).unwrap();
    // The echo of what is typed must not itself contain the watched text.
    p.run_command(r"printf 'all fine\n\033[31mBOO%s here\033[0m silent\nFAIL %d\n\033]9;Build done\007' M 42");

    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while events.len() < 3 {
        events.extend(std::mem::take(&mut *p.shared.events.lock().unwrap()));
        assert!(Instant::now() < deadline, "events so far: {events:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
    let watched: Vec<_> = events.iter().filter_map(|e| if let PaneEvent::Watch { pattern, line } = e { Some((pattern.as_str(), line.as_str())) } else { None }).collect();
    assert!(watched.contains(&("boom", "BOOM here silent")), "{events:?}");
    assert!(watched.contains(&(r"^FAIL \d+$", "FAIL 42")), "{events:?}");
    assert_eq!(watched.len(), 2, "the search without Alert stays quiet: {events:?}");
    assert!(events.iter().any(|e| matches!(e, PaneEvent::Notify { body, .. } if body == "Build done")), "{events:?}");

    // The recording holds plain text.
    let path = p.recording.clone().unwrap();
    let recorded = std::fs::read_to_string(&path).unwrap();
    assert!(recorded.contains("BOOM here silent") && recorded.contains("FAIL 42") && !recorded.contains('\x1b'), "{recorded}");
    p.set_recording(false, &cfg).unwrap();
    assert!(p.recording.is_none());

    // Scrollback is written once, and comes back (dimmed, with a banner) in a new terminal.
    p.save_scrollback(100);
    let saved = crate::session::load_scrollback(&p.uid).unwrap();
    assert!(saved.contains("FAIL 42"), "{saved}");
    let q = Pane::spawn(2, &s, Some(cfg.clone()), size, (8.0, 16.0), &ctx, Some(&saved)).unwrap();
    let text = all_text(&q.term.lock());
    assert!(text.contains("FAIL 42") && text.contains("restored from the previous session"), "{text}");
    crate::session::prune_scrollback(&[]);
    assert!(crate::session::load_scrollback(&p.uid).is_none());

    unsafe { std::env::remove_var("TERM_CONFIG_DIR") };
    let _ = std::fs::remove_dir_all(&cfg);
}

/// Keeping a command alive, and moving terminals between tabs.
#[test]
#[ignore]
fn reconnect_and_move_terminals() {
    use std::time::Duration;

    let _guard = CONFIG_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = std::env::temp_dir().join(format!("term-move-{}", std::process::id()));
    // SAFETY: tests touching TERM_CONFIG_DIR hold CONFIG_DIR_LOCK.
    unsafe { std::env::set_var("TERM_CONFIG_DIR", &cfg) };
    let mut h = egui_kittest::Harness::builder()
        .with_size([1100.0, 640.0])
        .build_eframe(|cc| crate::app::App::new(cc, crate::Args { layout: None, run: false, fresh: true, dir: Some("/tmp".into()) }));
    h.run_steps(2);

    // An ssh that dropped (exit 255) is retried; a clean exit or Ctrl+C ends the keeping alive.
    let secs = Duration::from_secs;
    assert_eq!(h.state_mut().finish_command("ssh host", Some(255), secs(5)), (true, false, true));
    assert_eq!(h.state_mut().finish_command("ssh host", Some(0), secs(5)), (true, false, false), "the retry was already scheduled");
    // Without an exit status it is not clear whether the command failed: offer, do not retry.
    assert_eq!(h.state_mut().finish_command("ssh host", None, secs(5)).1, true);
    // Something else that ended in this terminal is not the saved command.
    assert_eq!(h.state_mut().finish_command("ls", Some(255), secs(1)).2, true);

    // Three terminals in one tab, then moved around.
    for step in ["split_vertical", "split_horizontal"] {
        h.state_mut().script_step(step);
        h.run_steps(2);
    }
    assert_eq!(h.state().tab_sizes(), vec![3]);
    h.state_mut().script_step("move_pane:new");
    h.run_steps(2);
    assert_eq!(h.state().tab_sizes(), vec![2, 1], "the terminal went to a tab of its own");
    h.state_mut().script_step("move_pane:0");
    h.run_steps(2);
    assert_eq!(h.state().tab_sizes(), vec![3], "moving the only terminal of a tab removes that tab");
    h.state_mut().script_step("move_pane:new");
    h.run_steps(2);
    h.state_mut().script_step("move_pane:new");
    h.run_steps(2);
    assert_eq!(h.state().tab_sizes(), vec![2, 1], "a terminal alone in its tab stays put");

    unsafe { std::env::remove_var("TERM_CONFIG_DIR") };
    let _ = std::fs::remove_dir_all(&cfg);
}

/// A program (say, vim over ssh) copying to the clipboard with OSC 52 reaches the terminal's clipboard request.
#[test]
#[cfg(unix)]
fn osc52_clipboard_store_arrives() {
    use std::time::{Duration, Instant};

    use crate::pane::{Pane, TermSize};

    let ctx = eframe::egui::Context::default();
    let mut s = Settings::default();
    s.shell = "/bin/sh".into();
    let p = Pane::spawn(1, &s, Some(std::env::temp_dir()), TermSize { cols: 80, rows: 24 }, (8.0, 16.0), &ctx, None).unwrap();
    p.run_command(r"printf '\033]52;c;aGVsbG8gZnJvbSB0aGUgcmVtb3RlIGhvc3Q=\007'");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(text) = p.shared.clipboard_store.lock().unwrap().take() {
            assert_eq!(text, "hello from the remote host");
            return;
        }
        assert!(Instant::now() < deadline, "no clipboard request arrived");
        std::thread::sleep(Duration::from_millis(20));
    }
}
