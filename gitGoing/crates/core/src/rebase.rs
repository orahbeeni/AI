//! Rebase: onto another branch, or interactively (reorder, reword, squash,
//! fixup, drop). The work is done by the system `git`; for interactive rebases
//! our own executable stands in as git's sequence editor and hands it the todo
//! list the user built (see `gitcmd`).

use crate::gitcmd;
use git2::{Oid, Repository, RepositoryState, Sort};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Pick,
    Reword,
    Squash,
    Fixup,
    Drop,
}

impl Action {
    pub const ALL: [Action; 5] = [Action::Pick, Action::Reword, Action::Squash, Action::Fixup, Action::Drop];

    pub fn label(self) -> &'static str {
        match self {
            Action::Pick => "pick",
            Action::Reword => "reword",
            Action::Squash => "squash",
            Action::Fixup => "fixup",
            Action::Drop => "drop",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TodoItem {
    pub id: Oid,
    pub summary: String,
    pub action: Action,
    /// Full commit message; used by `Reword`.
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Stopped on conflicts; resolve, then continue, skip or abort.
    Conflicts,
}

/// Largest range offered for interactive rebasing.
pub const MAX_COMMITS: usize = 500;

fn e(err: git2::Error) -> String {
    err.message().to_owned()
}

/// Is a rebase in progress?
pub fn in_progress(repo: &Repository) -> bool {
    matches!(
        repo.state(),
        RepositoryState::Rebase | RepositoryState::RebaseInteractive | RepositoryState::RebaseMerge
    )
}

fn outcome_of(repo_path: &Path, result: Result<String, String>) -> Result<Outcome, String> {
    let stopped = Repository::discover(repo_path).map(|r| in_progress(&r)).unwrap_or(false);
    match result {
        Ok(_) if !stopped => Ok(Outcome::Done),
        Ok(_) => Ok(Outcome::Conflicts),
        Err(_) if stopped => Ok(Outcome::Conflicts),
        Err(e) => Err(e),
    }
}

/// `git rebase --autostash <onto>`.
pub fn rebase_onto(repo_path: &Path, onto: &str) -> Result<Outcome, String> {
    gitcmd::plain_name(onto)?;
    {
        let repo = Repository::discover(repo_path).map_err(e)?;
        crate::merge::ensure_clean_state(&repo).map_err(e)?;
    }
    let r = gitcmd::run(Some(repo_path), &["rebase", "--autostash", onto]);
    outcome_of(repo_path, r)
}

/// The commits from `from` (inclusive) up to HEAD, oldest first.
pub fn plan(repo_path: &Path, from: Oid) -> Result<Vec<TodoItem>, String> {
    let repo = Repository::discover(repo_path).map_err(e)?;
    crate::merge::ensure_clean_state(&repo).map_err(e)?;
    let head = repo.head().map_err(e)?.peel_to_commit().map_err(e)?;
    let start = repo.find_commit(from).map_err(e)?;
    let mut walk = repo.revwalk().map_err(e)?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE).map_err(e)?;
    walk.push(head.id()).map_err(e)?;
    if let Ok(parent) = start.parent_id(0) {
        walk.hide(parent).map_err(e)?;
    }
    let mut items = Vec::new();
    for oid in walk {
        let c = repo.find_commit(oid.map_err(e)?).map_err(e)?;
        if c.parent_count() > 1 {
            return Err("this range contains merge commits, which an interactive rebase would flatten".into());
        }
        if items.len() >= MAX_COMMITS {
            return Err(format!("more than {MAX_COMMITS} commits; pick a more recent starting commit"));
        }
        items.push(TodoItem {
            id: c.id(),
            summary: c.summary().unwrap_or("").to_owned(),
            action: Action::Pick,
            message: c.message().unwrap_or("").trim_end().to_owned(),
        });
    }
    if items.first().map(|i| i.id) != Some(from) {
        return Err("that commit is not on the current branch".into());
    }
    Ok(items)
}

fn todo_text(items: &[TodoItem], dir: &Path, amend_after: Option<(usize, &str)>) -> Result<String, String> {
    if matches!(items.first().map(|i| i.action), Some(Action::Squash | Action::Fixup)) {
        return Err("the first commit cannot be squashed or fixed up into anything".into());
    }
    let mut out = String::new();
    for (n, it) in items.iter().enumerate() {
        let sha = it.id.to_string();
        let short: String = it.summary.chars().take(60).collect::<String>().replace('\n', " ");
        match it.action {
            Action::Pick => out += &format!("pick {sha} {short}\n"),
            Action::Squash => out += &format!("squash {sha} {short}\n"),
            Action::Fixup => out += &format!("fixup {sha} {short}\n"),
            Action::Drop => out += &format!("drop {sha} {short}\n"),
            Action::Reword => {
                let file: PathBuf = dir.join(format!("msg-{n}.txt"));
                std::fs::write(&file, format!("{}\n", it.message.trim())).map_err(|e| e.to_string())?;
                out += &format!("pick {sha} {short}\nexec git commit --amend -q -F {}\n", gitcmd::shell_quote(&file));
            }
        }
        // After a whole squash group, give the combined commit its final message.
        if let Some((i, message)) = amend_after.filter(|(i, _)| *i == n) {
            let file: PathBuf = dir.join(format!("squash-msg-{i}.txt"));
            std::fs::write(&file, format!("{}\n", message.trim())).map_err(|e| e.to_string())?;
            out += &format!("exec git commit --amend -q -F {}\n", gitcmd::shell_quote(&file));
        }
    }
    Ok(out)
}

/// Start an interactive rebase of `from..HEAD` using `items` (oldest first).
pub fn start_interactive(repo_path: &Path, from: Oid, items: &[TodoItem]) -> Result<Outcome, String> {
    let exe = gitcmd::helper_exe().ok_or("interactive rebase needs the gitgoing executable as its editor")?;
    let editor = |todo: &Path| format!("{} --gg-sequence-editor {}", gitcmd::shell_quote(exe), gitcmd::shell_quote(todo));
    start_with(repo_path, from, items, None, editor)
}

fn start_with(
    repo_path: &Path,
    from: Oid,
    items: &[TodoItem],
    amend_after: Option<(usize, &str)>,
    editor: impl Fn(&Path) -> String,
) -> Result<Outcome, String> {
    let repo = Repository::discover(repo_path).map_err(e)?;
    crate::merge::ensure_clean_state(&repo).map_err(e)?;
    // A fresh, unpredictable directory: `create_dir` (not `create_dir_all`) fails
    // rather than reuse one somebody else created.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("gitgoing-rebase-{}-{nanos}", std::process::id()));
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let todo = dir.join("todo.txt");
    std::fs::write(&todo, todo_text(items, &dir, amend_after)?).map_err(|e| e.to_string())?;

    let start = repo.find_commit(from).map_err(e)?;
    let mut cmd = gitcmd::command(Some(repo_path));
    cmd.env("GIT_SEQUENCE_EDITOR", editor(&todo)).args(["rebase", "-i", "--autostash"]);
    match start.parent_id(0) {
        Ok(base) => cmd.arg(base.to_string()),
        Err(_) => cmd.arg("--root"),
    };
    let r = gitcmd::run_cmd(&mut cmd, "rebase");
    outcome_of(repo_path, r)
}

/// A conflict the squash is predicted to cause when commits are re-applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PredictedConflict {
    /// The commit whose changes will not apply cleanly.
    pub commit: Oid,
    pub summary: String,
    pub files: Vec<String>,
}

/// One commit in the order the rewritten history will have.
#[derive(Clone, Debug)]
pub struct PlannedCommit {
    pub id: Oid,
    pub summary: String,
    /// Part of the group being squashed into one commit.
    pub squashed: bool,
}

/// A validated request to squash some commits into one.
#[derive(Clone, Debug)]
pub struct SquashPlan {
    /// The oldest of the selected commits.
    pub from: Oid,
    pub count: usize,
    /// The selected commits' messages joined, oldest first: a starting point
    /// for the new message.
    pub message: String,
    /// No other commits lie between the selected ones, so nothing is reordered.
    pub consecutive: bool,
    /// The new history from `from` onward, oldest first.
    pub order: Vec<PlannedCommit>,
    /// The first conflict predicted by replaying the new order in memory.
    pub conflict: Option<PredictedConflict>,
    /// Why the conflict check could not run, if it could not.
    pub check_error: Option<String>,
    /// How many of the rewritten commits are already on the upstream branch.
    pub pushed: usize,
}

/// Selected commits first (in their original order), then everything else.
fn squash_order(items: &[TodoItem], ids: &[Oid]) -> (Vec<TodoItem>, usize) {
    let (mut group, rest): (Vec<_>, Vec<_>) = items.iter().cloned().partition(|i| ids.contains(&i.id));
    let k = group.len();
    for (n, it) in group.iter_mut().enumerate() {
        it.action = if n == 0 { Action::Pick } else { Action::Fixup };
    }
    group.extend(rest);
    (group, k)
}

/// Replay `order` onto the parent of its first commit entirely in memory (no
/// files, index or branches are touched) and report the first commit whose
/// changes do not apply cleanly.
fn predict_conflict(repo: &Repository, base_tree: Oid, order: &[Oid]) -> Result<Option<PredictedConflict>, String> {
    let empty = repo.treebuilder(None).map_err(e)?.write().map_err(e)?;
    let mut tip = base_tree;
    for &id in order {
        let c = repo.find_commit(id).map_err(e)?;
        let ancestor = match c.parent(0) {
            Ok(p) => p.tree_id(),
            Err(_) => empty,
        };
        let mut merged = repo
            .merge_trees(
                &repo.find_tree(ancestor).map_err(e)?,
                &repo.find_tree(tip).map_err(e)?,
                &c.tree().map_err(e)?,
                None,
            )
            .map_err(e)?;
        if merged.has_conflicts() {
            let mut files: Vec<String> = merged
                .conflicts()
                .map_err(e)?
                .filter_map(Result::ok)
                .filter_map(|x| x.our.or(x.their).or(x.ancestor))
                .map(|x| String::from_utf8_lossy(&x.path).into_owned())
                .collect();
            files.sort();
            files.dedup();
            return Ok(Some(PredictedConflict { commit: id, summary: c.summary().unwrap_or("").to_owned(), files }));
        }
        tip = merged.write_tree_to(repo).map_err(e)?;
    }
    Ok(None)
}

/// Check that `ids` can be squashed (all on the current branch, no merges in
/// the range), propose a combined message, and predict whether re-applying the
/// commits after the squash will conflict.
///
/// The selected commits need not be next to each other: the ones in between
/// are moved after the combined commit.
pub fn plan_squash(repo_path: &Path, ids: &[Oid]) -> Result<SquashPlan, String> {
    if ids.len() < 2 {
        return Err("select at least two commits to squash".into());
    }
    let repo = Repository::discover(repo_path).map_err(e)?;
    let head = repo.head().map_err(e)?.peel_to_commit().map_err(e)?;
    // Find the oldest selected commit by walking back from HEAD.
    let mut walk = repo.revwalk().map_err(e)?;
    walk.set_sorting(Sort::TOPOLOGICAL).map_err(e)?;
    walk.push(head.id()).map_err(e)?;
    let mut found = 0;
    let mut oldest = None;
    for oid in walk {
        let oid = oid.map_err(e)?;
        if ids.contains(&oid) {
            found += 1;
            oldest = Some(oid);
            if found == ids.len() {
                break;
            }
        }
    }
    let from = oldest.filter(|_| found == ids.len()).ok_or("the selected commits must all be on the current branch")?;
    let items = plan(repo_path, from)?;
    let k = ids.len();
    let consecutive = items[..k.min(items.len())].iter().all(|i| ids.contains(&i.id));
    let (ordered, _) = squash_order(&items, ids);

    let message = items
        .iter()
        .filter(|i| ids.contains(&i.id))
        .map(|i| i.message.trim())
        .collect::<Vec<_>>()
        .join("\n\n");
    let order = ordered
        .iter()
        .enumerate()
        .map(|(n, i)| PlannedCommit { id: i.id, summary: i.summary.clone(), squashed: n < k })
        .collect();

    // Consecutive squashes replay the same trees in the same order, so they
    // cannot conflict; only reordering needs checking.
    let (conflict, check_error) = if consecutive {
        (None, None)
    } else {
        let base_tree = match repo.find_commit(from).and_then(|c| c.parent(0)) {
            Ok(parent) => Ok(parent.tree_id()),
            // The oldest selected commit is the root: replay onto an empty tree.
            Err(_) => repo.treebuilder(None).and_then(|b| b.write()),
        };
        match base_tree
            .map_err(e)
            .and_then(|t| predict_conflict(&repo, t, &ordered.iter().map(|i| i.id).collect::<Vec<_>>()))
        {
            Ok(c) => (c, None),
            Err(err) => (None, Some(err)),
        }
    };
    let pushed = crate::remote::pushed_count(repo_path, from);
    Ok(SquashPlan { from, count: k, message, consecutive, order, conflict, check_error, pushed })
}

/// How many commits from `from` up to HEAD are already pushed (rewriting them needs a force push).
pub fn pushed_count_from(repo_path: &Path, from: Oid) -> usize {
    crate::remote::pushed_count(repo_path, from)
}

/// Combine the selected commits into one with `message`, rewriting everything
/// after them onto the result. Like an interactive rebase, it can stop on conflicts.
pub fn squash(repo_path: &Path, ids: &[Oid], message: &str) -> Result<Outcome, String> {
    let exe = gitcmd::helper_exe().ok_or("squashing needs the gitgoing executable as its editor")?;
    let editor = |todo: &Path| format!("{} --gg-sequence-editor {}", gitcmd::shell_quote(exe), gitcmd::shell_quote(todo));
    squash_with(repo_path, ids, message, editor)
}

fn squash_with(
    repo_path: &Path,
    ids: &[Oid],
    message: &str,
    editor: impl Fn(&Path) -> String,
) -> Result<Outcome, String> {
    if message.trim().is_empty() {
        return Err("the commit message is empty".into());
    }
    let p = plan_squash(repo_path, ids)?;
    let items = plan(repo_path, p.from)?;
    let (ordered, k) = squash_order(&items, ids);
    start_with(repo_path, p.from, &ordered, Some((k - 1, message)), editor)
}

pub fn continue_(repo_path: &Path) -> Result<Outcome, String> {
    let r = gitcmd::run(Some(repo_path), &["rebase", "--continue"]);
    outcome_of(repo_path, r)
}

pub fn skip(repo_path: &Path) -> Result<Outcome, String> {
    let r = gitcmd::run(Some(repo_path), &["rebase", "--skip"]);
    outcome_of(repo_path, r)
}

pub fn abort(repo_path: &Path) -> Result<(), String> {
    gitcmd::run(Some(repo_path), &["rebase", "--abort"]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{branches, status};

    fn write_commit(p: &Path, file: &str, content: &str, msg: &str) -> Oid {
        std::fs::write(p.join(file), content).unwrap();
        status::stage(p, &[file.into()]).unwrap();
        status::commit(p, msg).unwrap()
    }

    fn setup() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        write_commit(dir.path(), "a", "base\n", "base");
        let main = repo.head().unwrap().shorthand().unwrap().to_owned();
        (dir, main)
    }

    fn git_available() -> bool {
        gitcmd::run(None, &["--version"]).is_ok()
    }

    fn history(p: &Path) -> Vec<String> {
        let repo = Repository::open(p).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        walk.map(|o| repo.find_commit(o.unwrap()).unwrap().message().unwrap().trim().to_owned()).collect()
    }

    /// Stand-in for our executable: `cp <ours>` then git appends its todo path.
    fn cp_editor(todo: &Path) -> String {
        format!("cp {}", gitcmd::shell_quote(todo))
    }

    #[test]
    fn interactive_reorder_reword_squash_drop() {
        if !git_available() {
            return;
        }
        let (dir, _) = setup();
        let p = dir.path();
        let c1 = write_commit(p, "f1", "1\n", "one");
        write_commit(p, "f2", "2\n", "two");
        write_commit(p, "f3", "3\n", "three");
        write_commit(p, "f4", "4\n", "four");

        let mut items = plan(p, c1).unwrap();
        assert_eq!(items.iter().map(|i| i.summary.as_str()).collect::<Vec<_>>(), ["one", "two", "three", "four"]);
        items[1].action = Action::Squash; // two -> into one
        items[2].action = Action::Drop; // three gone
        items[3].action = Action::Reword;
        items[3].message = "FOUR (reworded)".into();
        let out = start_with(p, c1, &items, None, cp_editor).unwrap();
        assert_eq!(out, Outcome::Done);

        let h = history(p);
        assert_eq!(h.len(), 3, "base + squashed + reworded: {h:?}");
        assert_eq!(h[0], "FOUR (reworded)");
        assert!(h[1].contains("one") && h[1].contains("two"), "squash keeps both messages: {h:?}");
        assert!(p.join("f2").exists() && !p.join("f3").exists() && p.join("f4").exists());

        // Reordering: swap the last two commits.
        let repo = Repository::open(p).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let start = head.parent_id(0).unwrap();
        let mut items = plan(p, start).unwrap();
        items.reverse();
        assert_eq!(start_with(p, start, &items, None, cp_editor).unwrap(), Outcome::Done);
        let h = history(p);
        assert_eq!(h[1], "FOUR (reworded)", "order swapped: {h:?}");
        assert!(h[0].contains("one"));
    }

    #[test]
    fn squash_consecutive_commits_with_a_new_message() {
        if !git_available() {
            return;
        }
        let (dir, _) = setup();
        let p = dir.path();
        write_commit(p, "f1", "1\n", "one");
        let c2 = write_commit(p, "f2", "2\n", "two");
        let c3 = write_commit(p, "f3", "3\n", "three");
        let c4 = write_commit(p, "f4", "4\n", "four");
        write_commit(p, "f5", "5\n", "five");

        // Not consecutive is allowed, but flagged; touching different files, so no conflict.
        let sp = plan_squash(p, &[c2, c4]).unwrap();
        assert!(!sp.consecutive);
        assert!(sp.conflict.is_none() && sp.check_error.is_none());
        assert!(plan_squash(p, &[c2]).is_err());

        // The proposed message joins the originals, oldest first.
        let sp = plan_squash(p, &[c4, c2, c3]).unwrap(); // order of selection doesn't matter
        assert_eq!((sp.from, sp.count), (c2, 3));
        assert!(sp.consecutive);
        assert_eq!(sp.message, "two\n\nthree\n\nfour");

        assert!(squash_with(p, &[c2, c3, c4], "  ", cp_editor).is_err(), "empty message");
        assert_eq!(squash_with(p, &[c2, c3, c4], "Combined work", cp_editor).unwrap(), Outcome::Done);
        let h = history(p);
        assert_eq!(h, ["five", "Combined work", "one", "base"]);
        // Nothing is lost: every file is still there.
        for f in ["f1", "f2", "f3", "f4", "f5"] {
            assert!(p.join(f).exists(), "{f}");
        }
    }

    #[test]
    fn squashing_around_an_unselected_commit_in_the_middle() {
        if !git_available() {
            return;
        }
        let (dir, _) = setup();
        let p = dir.path();
        let a = write_commit(p, "fa", "a\n", "A");
        write_commit(p, "fb", "b\n", "B");
        let c = write_commit(p, "fc", "c\n", "C");
        write_commit(p, "fd", "d\n", "D");

        let sp = plan_squash(p, &[a, c]).unwrap();
        assert!(!sp.consecutive);
        assert_eq!(sp.order.iter().map(|x| x.summary.as_str()).collect::<Vec<_>>(), ["A", "C", "B", "D"]);
        assert_eq!(sp.order.iter().filter(|x| x.squashed).count(), 2);
        assert!(sp.conflict.is_none());
        assert_eq!(squash_with(p, &[a, c], "A+C", cp_editor).unwrap(), Outcome::Done);
        assert_eq!(history(p), ["D", "B", "A+C", "base"]);
    }

    #[test]
    fn predicts_a_conflict_before_anything_changes() {
        if !git_available() {
            return;
        }
        let (dir, _) = setup();
        let p = dir.path();
        let a = write_commit(p, "x", "1\n", "A: create x");
        write_commit(p, "x", "1\n2\n", "B: add a line to x");
        let c = write_commit(p, "x", "1\n2\n3\n", "C: add another line to x");
        write_commit(p, "other", "o\n", "D");
        let head_before = Repository::open(p).unwrap().head().unwrap().target();

        // C builds on B, so moving C ahead of B cannot apply cleanly.
        let sp = plan_squash(p, &[a, c]).unwrap();
        let predicted = sp.conflict.expect("a conflict is predicted");
        assert_eq!(predicted.commit, c);
        assert_eq!(predicted.files, ["x"]);
        // The prediction touched nothing.
        let repo = Repository::open(p).unwrap();
        assert_eq!(repo.head().unwrap().target(), head_before);
        assert_eq!(repo.state(), RepositoryState::Clean);
        assert!(status::status(p).unwrap().unstaged.is_empty());

        // And it is right: running the squash really stops on a conflict.
        assert_eq!(squash_with(p, &[a, c], "A+C", cp_editor).unwrap(), Outcome::Conflicts);
        assert!(in_progress(&Repository::open(p).unwrap()));
        abort(p).unwrap();
        assert_eq!(Repository::open(p).unwrap().head().unwrap().target(), head_before);
    }

    #[test]
    fn first_item_cannot_be_squash() {
        let (dir, _) = setup();
        let p = dir.path();
        let c = write_commit(p, "x", "x\n", "x");
        let mut items = plan(p, c).unwrap();
        items[0].action = Action::Fixup;
        assert!(start_with(p, c, &items, None, cp_editor).is_err());
    }

    #[test]
    fn rebase_onto_with_conflict_abort_and_continue() {
        if !git_available() {
            return;
        }
        let (dir, main) = setup();
        let p = dir.path();
        branches::create_and_checkout(p, "feature").unwrap();
        write_commit(p, "a", "feature\n", "feature edit");
        branches::checkout(p, &main).unwrap();
        write_commit(p, "a", "main\n", "main edit");
        branches::checkout(p, "feature").unwrap();

        assert_eq!(rebase_onto(p, &main).unwrap(), Outcome::Conflicts);
        assert_eq!(status::status(p).unwrap().operation, Some("Rebase"));
        assert!(status::commit(p, "no").is_err(), "must not commit manually during a rebase");
        abort(p).unwrap();
        assert!(!in_progress(&Repository::open(p).unwrap()));
        assert_eq!(std::fs::read_to_string(p.join("a")).unwrap(), "feature\n");

        assert_eq!(rebase_onto(p, &main).unwrap(), Outcome::Conflicts);
        crate::merge::resolve(p, "a", crate::merge::Side::Theirs).unwrap();
        assert_eq!(continue_(p).unwrap(), Outcome::Done);
        let h = history(p);
        assert_eq!(h[0], "feature edit");
        assert_eq!(h[1], "main edit");
    }

    #[test]
    fn rejects_merges_and_foreign_commits() {
        let (dir, main) = setup();
        let p = dir.path();
        branches::create_and_checkout(p, "side").unwrap();
        let side = write_commit(p, "s", "s\n", "side");
        branches::checkout(p, &main).unwrap();
        let m = write_commit(p, "m", "m\n", "main");
        assert!(plan(p, side).is_err(), "not on the current branch");
        crate::merge::merge(p, "side").unwrap();
        assert!(plan(p, m).unwrap_err().contains("merge commits"));
    }
}
