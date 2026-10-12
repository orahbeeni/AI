//! End-to-end check that git really calls our executable back as its sequence
//! editor, with correct quoting, when an interactive rebase runs.

use gitgoing_core::{gitcmd, rebase, status};
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git").arg("-C").arg(dir).args(args).status().expect("git").success();
    assert!(ok, "git {args:?} failed");
}

#[test]
fn interactive_rebase_uses_our_binary_as_sequence_editor() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    gitcmd::set_helper_exe(env!("CARGO_BIN_EXE_gitgoing").into());

    // A path with a space exercises the shell quoting.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("my repo");
    std::fs::create_dir(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.name", "T"]);
    git(&dir, &["config", "user.email", "t@example.com"]);
    let mut ids = Vec::new();
    for n in 1..=3 {
        std::fs::write(dir.join(format!("f{n}")), format!("{n}\n")).unwrap();
        status::stage(&dir, &[format!("f{n}")]).unwrap();
        ids.push(status::commit(&dir, &format!("commit {n}")).unwrap());
    }

    let mut items = rebase::plan(&dir, ids[0]).unwrap();
    items[1].action = rebase::Action::Fixup;
    items[2].action = rebase::Action::Reword;
    items[2].message = "third, reworded".into();
    assert_eq!(rebase::start_interactive(&dir, ids[0], &items).unwrap(), rebase::Outcome::Done);

    let out = Command::new("git").arg("-C").arg(&dir).args(["log", "--format=%s"]).output().unwrap();
    let log = String::from_utf8(out.stdout).unwrap();
    assert_eq!(log.lines().collect::<Vec<_>>(), ["third, reworded", "commit 1"]);
}
