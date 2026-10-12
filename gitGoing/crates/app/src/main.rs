mod app;
mod askpass;
mod changes;
mod clonedlg;
mod conflictwin;
mod detail;
mod diffview;
mod filetree;
mod icons;
mod settings;
mod sidebar;
mod syntax;
mod timefmt;

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
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("gitgoing")
            .with_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "gitgoing",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, initial, resolve.clone())))),
    )
}
