//! Change the date of a commit without touching its content.
//!
//! The commit is copied with the same tree and parents but a new date. Commits
//! after it on the current branch are copied too (their parent's hash changed),
//! again with their own tree, message, authors and dates untouched, so nothing
//! can be added to or dropped from any commit and no conflicts can arise.
//! The index and working tree are never touched: the tip's tree is unchanged.

use git2::{Commit, Oid, Repository, Signature, Sort, Time};
use std::collections::{HashMap, HashSet};
use std::path::Path;

fn err(msg: impl AsRef<str>) -> git2::Error {
    git2::Error::from_str(msg.as_ref())
}

/// What changing a commit's date would do.
#[derive(Clone, Debug)]
pub struct Plan {
    pub branch: String,
    pub summary: String,
    /// Author and committer time of the commit, with their UTC offsets (minutes).
    pub author_time: (i64, i32),
    pub committer_time: (i64, i32),
    pub author: String,
    /// The commit's author is the configured user.
    pub author_is_me: bool,
    /// Commits that get new hashes, the chosen one included.
    pub rewritten: usize,
    /// Remote-tracking branches that already contain the commit.
    pub pushed_to: Vec<String>,
    /// Other local branches, tags or the stash that contain the commit and would keep the old copy.
    pub also_on: Vec<String>,
}

fn current_branch(repo: &Repository) -> Result<(String, Oid), git2::Error> {
    if repo.head_detached()? {
        return Err(err("check out a branch first: the date can only be changed on the current branch"));
    }
    let head = repo.head().map_err(|_| err("this repository has no commits yet"))?;
    let name = head.name().ok_or_else(|| err("the current branch has an unusable name"))?.to_owned();
    let tip = head.target().ok_or_else(|| err("the current branch has no commits"))?;
    Ok((name, tip))
}

/// The commits to copy, parents before children: `id` and everything on the way to `tip` that builds on it.
fn rewrite_set(repo: &Repository, tip: Oid, id: Oid) -> Result<Vec<Oid>, git2::Error> {
    if tip != id && !repo.graph_descendant_of(tip, id)? {
        return Err(err("this commit is not on the current branch"));
    }
    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?;
    walk.push(tip)?;
    for p in repo.find_commit(id)?.parent_ids() {
        walk.hide(p)?;
    }
    let mut built_on_id: HashSet<Oid> = HashSet::from([id]);
    let mut out = Vec::new();
    for oid in walk {
        let oid = oid?;
        let c = repo.find_commit(oid)?;
        if oid == id || c.parent_ids().any(|p| built_on_id.contains(&p)) {
            built_on_id.insert(oid);
            out.push(oid);
        }
    }
    Ok(out)
}

pub fn plan(repo_path: &Path, id: Oid) -> Result<Plan, git2::Error> {
    let repo = Repository::discover(repo_path)?;
    crate::merge::ensure_clean_state(&repo)?;
    let (branch_ref, tip) = current_branch(&repo)?;
    let set = rewrite_set(&repo, tip, id)?;
    let commit = repo.find_commit(id)?;
    let (mut pushed_to, mut also_on) = (Vec::new(), Vec::new());
    for r in repo.references()? {
        let r = r?;
        let Some(name) = r.name() else { continue };
        if name == branch_ref || r.symbolic_target().is_some() {
            continue;
        }
        let Ok(target) = r.peel_to_commit().map(|c| c.id()) else { continue };
        if target != id && !repo.graph_descendant_of(target, id)? {
            continue;
        }
        if let Some(short) = name.strip_prefix("refs/remotes/") {
            pushed_to.push(short.to_owned());
        } else {
            also_on.push(name.trim_start_matches("refs/heads/").trim_start_matches("refs/tags/").to_owned());
        }
    }
    let (a, c) = (commit.author(), commit.committer());
    let me = repo.signature().ok().and_then(|s| s.email().map(str::to_owned));
    Ok(Plan {
        branch: branch_ref.trim_start_matches("refs/heads/").to_owned(),
        summary: commit.summary().unwrap_or("").to_owned(),
        author_time: (a.when().seconds(), a.when().offset_minutes()),
        committer_time: (c.when().seconds(), c.when().offset_minutes()),
        author: a.name().unwrap_or("").to_owned(),
        author_is_me: me.is_some() && a.email().map(str::to_owned) == me,
        rewritten: set.len(),
        pushed_to,
        also_on,
    })
}

/// Turn what the user typed into a Unix time and UTC offset (minutes), using git's own date
/// parser and the local time zone. Empty text means "now".
pub fn parse_when(repo_path: &Path, text: &str) -> Result<(i64, i32), String> {
    let mut cmd = crate::gitcmd::command(Some(repo_path));
    cmd.args(["var", "GIT_COMMITTER_IDENT"])
        .env("GIT_COMMITTER_NAME", "x")
        .env("GIT_COMMITTER_EMAIL", "x@example.com");
    if !text.trim().is_empty() {
        cmd.env("GIT_COMMITTER_DATE", text.trim());
    }
    let out = crate::gitcmd::run_cmd(&mut cmd, "var").map_err(|e| format!("'{}' is not a date git understands ({e})", text.trim()))?;
    let bad = || format!("could not read the date from '{}'", out.trim());
    let rest = out.trim().rsplit_once('>').ok_or_else(bad)?.1;
    let mut parts = rest.split_whitespace();
    let secs: i64 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(bad)?;
    let tz = parts.next().ok_or_else(bad)?;
    let sign = if tz.starts_with('-') { -1 } else { 1 };
    let digits = tz.trim_start_matches(['+', '-']);
    if digits.len() != 4 {
        return Err(bad());
    }
    let (h, m): (i32, i32) = (digits[..2].parse().map_err(|_| bad())?, digits[2..].parse().map_err(|_| bad())?);
    Ok((secs, sign * (h * 60 + m)))
}

fn with_time(base: &Signature, when: (i64, i32)) -> Result<Signature<'static>, git2::Error> {
    Signature::new(base.name().unwrap_or(""), base.email().unwrap_or(""), &Time::new(when.0, when.1))
}

/// Give commit `id` the new date (`author` and/or `committer`), recreating the commits built on it.
/// With `purge`, every trace of the old commits is then deleted from this repository (reflogs and
/// objects). That is refused if a remote-tracking branch already has the commit.
pub fn redate(repo_path: &Path, id: Oid, when: (i64, i32), author: bool, committer: bool, purge: bool) -> Result<(), git2::Error> {
    crate::gitlog::equiv(
        repo_path,
        format!("git commit-tree <same tree and parents> (new date) for {id} and the commits built on it; git update-ref <branch> <new tip>"),
    );
    if !author && !committer {
        return Err(err("choose the author date, the committer date, or both"));
    }
    let repo = Repository::discover(repo_path)?;
    crate::merge::ensure_clean_state(&repo)?;
    let plan = plan(repo_path, id)?;
    if purge && !plan.pushed_to.is_empty() {
        return Err(err(format!("'{}' already has this commit, so it cannot be removed everywhere", plan.pushed_to[0])));
    }
    let (branch_ref, tip) = current_branch(&repo)?;
    let set = rewrite_set(&repo, tip, id)?;

    let mut new_of: HashMap<Oid, Oid> = HashMap::new();
    for &oid in &set {
        let old = repo.find_commit(oid)?;
        let parents = old
            .parent_ids()
            .map(|p| repo.find_commit(new_of.get(&p).copied().unwrap_or(p)))
            .collect::<Result<Vec<_>, _>>()?;
        let parent_refs: Vec<&Commit> = parents.iter().collect();
        let (mut a, mut c) = (old.author().to_owned(), old.committer().to_owned());
        if oid == id {
            if author {
                a = with_time(&a, when)?;
            }
            if committer {
                c = with_time(&c, when)?;
            }
        }
        let msg = old.message_raw().ok_or_else(|| err("a commit message is not valid UTF-8"))?;
        let new = repo.commit(None, &a, &c, msg, &old.tree()?, &parent_refs)?;
        new_of.insert(oid, new);
    }
    let new_tip = new_of.get(&tip).copied().ok_or_else(|| err("nothing was rewritten"))?;
    if repo.find_commit(new_tip)?.tree_id() != repo.find_commit(tip)?.tree_id() {
        return Err(err("internal check failed: the new branch tip would differ from the old one"));
    }
    repo.reference(&branch_ref, new_tip, true, "gitgoing: change commit date")?;

    if purge {
        let old: HashSet<Oid> = set.into_iter().collect();
        if let Ok(orig) = repo.find_reference("ORIG_HEAD") {
            if orig.target().is_some_and(|t| old.contains(&t)) {
                let mut orig = orig;
                orig.delete()?;
            }
        }
        let git = |args: &[&str]| crate::gitcmd::run(Some(repo_path), args).map_err(|e| err(e));
        git(&["reflog", "expire", "--expire-unreachable=now", "--all"])?;
        git(&["gc", "--prune=now", "--quiet"])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three commits a <- b <- c on the default branch; returns their ids.
    fn repo_abc(p: &Path) -> (Repository, [Oid; 3]) {
        let repo = Repository::init(p).unwrap();
        let mut ids = Vec::new();
        for (i, name) in ["a", "b", "c"].iter().enumerate() {
            std::fs::write(p.join(name), name).unwrap();
            let mut idx = repo.index().unwrap();
            idx.add_path(Path::new(name)).unwrap();
            idx.write().unwrap();
            let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
            let sig = Signature::new("T", "t@example.com", &Time::new(1_700_000_000 + i as i64 * 1000, 120)).unwrap();
            let parents: Vec<Commit> = ids.last().map(|&id| repo.find_commit(id).unwrap()).into_iter().collect();
            let refs: Vec<&Commit> = parents.iter().collect();
            ids.push(repo.commit(Some("HEAD"), &sig, &sig, name, &tree, &refs).unwrap());
        }
        (repo, [ids[0], ids[1], ids[2]])
    }

    #[test]
    fn middle_commit_keeps_trees_and_rewrites_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, [a, b, c]) = repo_abc(dir.path());
        let pl = plan(dir.path(), b).unwrap();
        assert_eq!(pl.rewritten, 2);
        redate(dir.path(), b, (1_800_000_000, 0), true, true, false).unwrap();

        let new_c = repo.head().unwrap().peel_to_commit().unwrap();
        assert_ne!(new_c.id(), c);
        assert_eq!(new_c.tree_id(), repo.find_commit(c).unwrap().tree_id(), "tip content is identical");
        assert_eq!(new_c.author().when().seconds(), 1_700_002_000, "later commits keep their own dates");
        let new_b = new_c.parent(0).unwrap();
        assert_eq!(new_b.tree_id(), repo.find_commit(b).unwrap().tree_id());
        assert_eq!((new_b.author().when().seconds(), new_b.committer().when().seconds()), (1_800_000_000, 1_800_000_000));
        assert_eq!(new_b.message(), Some("b"));
        assert_eq!(new_b.parent_id(0).unwrap(), a, "earlier commits are untouched");
    }

    #[test]
    fn author_only_and_purge_removes_old_objects() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, [_, _, c]) = repo_abc(dir.path());
        redate(dir.path(), c, (1_800_000_000, 0), true, false, true).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.author().when().seconds(), 1_800_000_000);
        assert_eq!(head.committer().when().seconds(), 1_700_002_000, "committer date untouched");
        let repo = Repository::open(dir.path()).unwrap();
        assert!(repo.find_commit(c).is_err(), "the old commit is gone from the repository");
        assert!(repo.reflog("HEAD").unwrap().iter().all(|e| e.id_new() != c && e.id_old() != c));
    }

    #[test]
    fn refuses_detached_and_foreign_commits() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, [a, _, c]) = repo_abc(dir.path());
        repo.set_head_detached(c).unwrap();
        assert!(plan(dir.path(), a).is_err());
    }

    #[test]
    fn parses_dates_the_way_git_does() {
        let dir = tempfile::tempdir().unwrap();
        repo_abc(dir.path());
        let (t, _) = parse_when(dir.path(), "2026-01-15 10:30").unwrap();
        assert!(t > 1_768_000_000 && t < 1_769_000_000, "{t}");
        assert!(parse_when(dir.path(), "").is_ok());
        assert!(parse_when(dir.path(), "not a date at all").is_err());
    }
}
