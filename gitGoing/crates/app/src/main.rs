mod app;
mod assets;
mod askpass;
mod changes;
mod clonedlg;
mod comparewin;
mod conflictwin;
mod consolewin;
mod detail;
mod diffview;
mod filemenu;
mod filerows;
mod filetree;
mod icons;
mod intro;
mod linehist;
mod settings;
mod sidebar;
mod startup;
mod syntax;
mod timefmt;
mod toasts;

fn main() -> eframe::Result {
    // Helper roles: git and ssh start this same executable to ask for
    // credentials, or to hand us a rebase todo list to rewrite.
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.get(1).is_some_and(|a| a == "--gg-sequence-editor") {
        // `<exe> --gg-sequence-editor <our-todo> <git-todo>`: replace git's todo with ours.
        let ok = args.len() == 4 && std::fs::copy(&args[2], &args[3]).is_ok();
        std::process::exit(if ok { 0 } else { 1 });
    }
    if std::env::var_os(gitgoing_core::gitcmd::ASKPASS_ENV).is_some() {
        askpass::run();
    }
    if let Ok(exe) = std::env::current_exe() {
        gitgoing_core::gitcmd::set_helper_exe(exe);
    }

    // Optional first argument: repository path. Defaults to the current dir.
    let initial = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    // `gitgoing <repo> --resolve <file>` opens the conflict editor straight away.
    let resolve = args.iter().position(|a| a == "--resolve").and_then(|i| args.get(i + 1)).map(|s| s.to_string_lossy().into_owned());
    // The intro plays in a borderless window of its own size, which then becomes the
    // normal window. Whether it plays, and where the window goes afterwards, must be
    // known before the window exists, hence the small files in `startup`.
    startup::forget_eframe_window();
    let intro = startup::intro_enabled() && !args.iter().any(|a| a == "--no-splash");
    let geom = startup::load_geom();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("gitgoing")
        // Wayland app_id / X11 class: lets the shell match the window to gitgoing.desktop
        .with_app_id("gitgoing")
        .with_icon(assets::window_icon());
    if intro {
        viewport = viewport
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size(startup::SPLASH_SIZE);
    } else if let Some(g) = &geom {
        viewport = viewport.with_inner_size(g.size).with_maximized(g.maximized);
        if let Some(p) = g.pos {
            viewport = viewport.with_position(p);
        }
    } else {
        viewport = viewport.with_inner_size(startup::DEFAULT_SIZE);
    }
    let options = eframe::NativeOptions {
        viewport,
        // We save and restore the window ourselves (see `startup`), so the intro's
        // borderless size is never remembered as the window's size.
        persist_window: false,
        centered: intro || geom.is_none_or(|g| g.pos.is_none()),
        ..Default::default()
    };
    let console = args.iter().any(|a| a == "--console");
    eframe::run_native(
        "gitgoing",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, initial, resolve.clone(), console, intro, geom)))),
    )
}
