//! Where the lines of a diff's old side came from: the commit that last changed each of them
//! (blame), and the full history of a range of lines (`git log -L`). Used to check, before
//! removing or changing a line, why it was added.

use std::collections::HashMap;
use std::path::Path;

use git2::{DiffOptions, Oid, Repository};

/// The old side of a diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OldSide {
    /// A commit's version of the file: a commit's parent, or HEAD for staged changes.
    Commit(Oid),
    /// The staged version (the old side of unstaged changes).
    Index,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineOrigin {
    pub id: Oid,
    pub author: String,
    pub time: i64,
    pub summary: String,
    /// Ticket key in the commit's summary, e.g. "ECOMD-4821".
    pub ticket: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Commit(LineOrigin),
    /// The line only exists in the staged version.
    Staged,
}

/// Origins of the old side's lines, keyed by old-side line number.
#[derive(Clone, Debug, Default)]
pub struct Origins {
    pub lines: HashMap<u32, Origin>,
    /// The commit whose version the line numbers refer to, for [`line_history`].
    pub rev: Option<Oid>,
    /// Old-side line number -> line number in `rev` (differs only for the staged version).
    pub rev_line: HashMap<u32, u32>,
    /// The path in `rev`.
    pub path: String,
    /// The ticket being worked on: from the branch name for working changes, from the commit's
    /// message for a commit. Lines added for another ticket are pointed out.
    pub context_ticket: Option<String>,
}

impl Origins {
    /// `rev` line numbers covering the old-side lines `start..=end`, if any of them exist in `rev`.
    pub fn rev_range(&self, start: u32, end: u32) -> Option<(u32, u32)> {
        let mapped: Vec<u32> = (start..=end).filter_map(|n| self.rev_line.get(&n).copied()).collect();
        Some((*mapped.iter().min()?, *mapped.iter().max()?))
    }
}

/// A ticket key such as "ECOMD-5610" in a branch name or commit message: capital letters (and
/// digits after the first), a dash, then digits.
pub fn ticket(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let starts_word = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        if starts_word && b[i].is_ascii_uppercase() {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_uppercase() || b[j].is_ascii_digit()) {
                j += 1;
            }
            if j - i >= 2 && j + 1 < b.len() && b[j] == b'-' && b[j + 1].is_ascii_digit() {
                let mut k = j + 1;
                while k < b.len() && b[k].is_ascii_digit() {
                    k += 1;
                }
                // Technical names shaped like keys (UTF-8, SHA-256, RFC-2616…) are not tickets.
                const NOT_TICKETS: &[&str] = &["UTF", "UCS", "ISO", "SHA", "MD", "RFC", "HTTP", "TLS", "SSL", "ES", "ECMA", "CP", "WIN", "IPV", "MP", "H", "X"];
                let prefix = &text[i..j];
                if (k == b.len() || !b[k].is_ascii_alphabetic()) && !NOT_TICKETS.contains(&prefix) {
                    return Some(text[i..k].to_owned());
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

/// Blame of `path` as of `rev`, from the system `git blame` (libgit2's blame takes minutes on a
/// repository with a long history; git itself, with its commit-graph, takes well under a second).
fn blame_at(repo_path: &Path, path: &str, rev: Oid) -> Result<HashMap<u32, LineOrigin>, git2::Error> {
    let rev_s = rev.to_string();
    let out = crate::gitcmd::run(Some(repo_path), &["blame", "--porcelain", &rev_s, "--", path]).map_err(|e| git2::Error::from_str(&e))?;
    Ok(parse_porcelain(&out))
}

/// Parse `git blame --porcelain`: a header `<sha> <orig line> <final line> [<group size>]` per line,
/// with the commit's author, time and summary the first time that commit appears.
fn parse_porcelain(out: &str) -> HashMap<u32, LineOrigin> {
    let mut info: HashMap<Oid, (String, i64, String)> = HashMap::new();
    let mut lines: Vec<(u32, Oid)> = Vec::new();
    let mut current: Option<Oid> = None;
    for l in out.lines() {
        if l.starts_with('\t') {
            continue; // the line's own text
        }
        let mut it = l.split(' ');
        let first = it.next().unwrap_or("");
        if first.len() == 40 && first.bytes().all(|b| b.is_ascii_hexdigit()) {
            let (Some(_orig), Some(fin)) = (it.next(), it.next().and_then(|n| n.parse::<u32>().ok())) else { continue };
            if let Ok(id) = Oid::from_str(first) {
                current = Some(id);
                lines.push((fin, id));
                info.entry(id).or_default();
            }
            continue;
        }
        let Some(id) = current else { continue };
        let value = l.split_once(' ').map(|(_, v)| v).unwrap_or("");
        let e = info.entry(id).or_default();
        match first {
            "author" => e.0 = value.to_owned(),
            "author-time" => e.1 = value.parse().unwrap_or(0),
            "summary" => e.2 = value.to_owned(),
            _ => {}
        }
    }
    lines
        .into_iter()
        .map(|(n, id)| {
            let (author, time, summary) = info.get(&id).cloned().unwrap_or_default();
            let ticket = ticket(&summary);
            (n, LineOrigin { id, author, time, summary, ticket })
        })
        .collect()
}

/// Staged line number -> HEAD line number, for every staged line that also exists in HEAD.
fn index_to_head(repo: &Repository, path: &str, head_lines: u32, index_lines: u32) -> Result<HashMap<u32, u32>, git2::Error> {
    let head_tree = repo.head()?.peel_to_tree()?;
    let index = repo.index()?;
    let mut opts = DiffOptions::new();
    opts.pathspec(path).disable_pathspec_match(true).context_lines(0);
    let diff = repo.diff_tree_to_index(Some(&head_tree), Some(&index), Some(&mut opts))?;
    // Hunks as (old_start, old_count, new_start, new_count); with no context, every line
    // outside them is unchanged and only shifted.
    let mut hunks: Vec<(u32, u32, u32, u32)> = Vec::new();
    diff.foreach(
        &mut |_, _| true,
        None,
        Some(&mut |_, h| {
            hunks.push((h.old_start(), h.old_lines(), h.new_start(), h.new_lines()));
            true
        }),
        None,
    )?;
    let mut map = HashMap::new();
    let (mut old, mut new) = (1u32, 1u32);
    let copy_until = |map: &mut HashMap<u32, u32>, old: &mut u32, new: &mut u32, new_end: u32| {
        while *new < new_end && *new <= index_lines && *old <= head_lines {
            map.insert(*new, *old);
            *new += 1;
            *old += 1;
        }
    };
    for (os, oc, ns, nc) in hunks {
        // A pure insertion's new_start is the line it starts at; a pure deletion's points
        // at the line before.
        let new_start = if nc == 0 { ns + 1 } else { ns };
        copy_until(&mut map, &mut old, &mut new, new_start);
        old = if oc == 0 { os + 1 } else { os + oc };
        new = new_start + nc;
    }
    copy_until(&mut map, &mut old, &mut new, u32::MAX);
    Ok(map)
}

/// The origin of every line on the old side of a diff of `path`.
pub fn origins(repo_path: &Path, path: &str, side: OldSide) -> Result<Origins, git2::Error> {
    match side {
        OldSide::Commit(rev) => {
            let lines = blame_at(repo_path, path, rev)?;
            let rev_line = lines.keys().map(|&n| (n, n)).collect();
            Ok(Origins {
                lines: lines.into_iter().map(|(n, o)| (n, Origin::Commit(o))).collect(),
                rev: Some(rev),
                rev_line,
                path: path.to_owned(),
                context_ticket: None,
            })
        }
        OldSide::Index => {
            let repo = Repository::discover(repo_path)?;
            let index = repo.index()?;
            let staged_text = match index.get_path(Path::new(path), 0) {
                Some(e) => repo.find_blob(e.id)?.content().to_vec(),
                None => return Ok(Origins::default()),
            };
            let index_lines = String::from_utf8_lossy(&staged_text).lines().count() as u32;
            let head = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
            let (blame, head_id) = match &head {
                Some(c) if c.tree()?.get_path(Path::new(path)).is_ok() => (blame_at(repo_path, path, c.id())?, Some(c.id())),
                // New in the index: nothing is committed yet.
                _ => (HashMap::new(), None),
            };
            let map = if head_id.is_some() { index_to_head(&repo, path, blame.len() as u32, index_lines)? } else { HashMap::new() };
            let mut lines = HashMap::new();
            for n in 1..=index_lines {
                let o = map.get(&n).and_then(|h| blame.get(h)).cloned().map(Origin::Commit).unwrap_or(Origin::Staged);
                lines.insert(n, o);
            }
            Ok(Origins { lines, rev: head_id, rev_line: map, path: path.to_owned(), context_ticket: None })
        }
    }
}

/// Origins for the diff of a staged (old side HEAD) or unstaged (old side the index) file.
pub fn for_working_changes(repo_path: &Path, path: &str, staged: bool) -> Result<Origins, git2::Error> {
    let repo = Repository::discover(repo_path)?;
    let head = repo.head().ok();
    let branch_ticket = head.as_ref().filter(|h| h.is_branch()).and_then(|h| h.shorthand()).and_then(ticket);
    let mut o = if staged {
        match head.and_then(|h| h.peel_to_commit().ok()) {
            Some(c) if c.tree()?.get_path(Path::new(path)).is_ok() => origins(repo_path, path, OldSide::Commit(c.id()))?,
            _ => Origins::default(), // a new file: no old side
        }
    } else {
        origins(repo_path, path, OldSide::Index)?
    };
    o.context_ticket = branch_ticket;
    Ok(o)
}

/// Origins for a file's diff in `commit` (old side: its first parent; `old_path` if renamed).
pub fn for_commit(repo_path: &Path, commit: Oid, old_path: &str) -> Result<Origins, git2::Error> {
    let repo = Repository::discover(repo_path)?;
    let c = repo.find_commit(commit)?;
    let Ok(parent) = c.parent(0) else { return Ok(Origins::default()) };
    if parent.tree()?.get_path(Path::new(old_path)).is_err() {
        return Ok(Origins::default()); // added in this commit
    }
    let mut o = origins(repo_path, old_path, OldSide::Commit(parent.id()))?;
    o.context_ticket = ticket(c.message().unwrap_or(""));
    Ok(o)
}

/// One commit in a line range's history, with the part of its diff touching those lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineCommit {
    pub id: Oid,
    pub author: String,
    pub time: i64,
    pub summary: String,
    pub message: String,
    pub ticket: Option<String>,
    /// Diff lines starting with '@@', '+', '-' or ' '.
    pub diff: Vec<String>,
}

/// Every commit that changed lines `start..=end` of `path` as of `rev`, newest first
/// (`git log -L`), at most `limit`.
pub fn line_history(repo_path: &Path, rev: Oid, path: &str, start: u32, end: u32, limit: usize) -> Result<Vec<LineCommit>, String> {
    let range = format!("-L{start},{end}:{path}");
    let rev_s = rev.to_string();
    let n = format!("-n{limit}");
    let out = crate::gitcmd::run(Some(repo_path), &["log", "--no-color", "--format=%x1e%H", &n, &range, &rev_s])?;
    let repo = Repository::discover(repo_path).map_err(|e| e.message().to_owned())?;
    let mut list = Vec::new();
    for chunk in out.split('\u{1e}').filter(|c| !c.trim().is_empty()) {
        let mut it = chunk.lines();
        let Some(id) = it.next().and_then(|h| Oid::from_str(h.trim()).ok()) else { continue };
        let Ok(c) = repo.find_commit(id) else { continue };
        let diff: Vec<String> = it
            .skip_while(|l| !l.starts_with("@@"))
            .filter(|l| !l.starts_with("diff --git") && !l.starts_with("--- ") && !l.starts_with("+++ "))
            .map(str::to_owned)
            .collect();
        let summary = c.summary().unwrap_or("").to_owned();
        list.push(LineCommit {
            id,
            author: c.author().name().unwrap_or("").to_owned(),
            time: c.time().seconds(),
            ticket: ticket(c.message().unwrap_or("")),
            summary,
            message: c.message().unwrap_or("").trim_end().to_owned(),
            diff,
        });
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn commit(repo: &Repository, dir: &Path, body: &str, msg: &str) -> Oid {
        fs::write(dir.join("f.txt"), body).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("f.txt")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Dev", "d@x").unwrap();
        let parents: Vec<git2::Commit> = repo.head().ok().and_then(|h| h.peel_to_commit().ok()).into_iter().collect();
        let refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &refs).unwrap()
    }

    fn origin_summary(o: &Origins, line: u32) -> String {
        match o.lines.get(&line) {
            Some(Origin::Commit(c)) => c.summary.clone(),
            Some(Origin::Staged) => "staged".into(),
            None => "-".into(),
        }
    }

    #[test]
    fn porcelain_blame_is_parsed() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let out = format!(
            "{a} 1 1 2\nauthor Jane Smith\nauthor-mail <j@x>\nauthor-time 1700000000\nsummary ECOMD-4821 first\nfilename f\n\tone\n{a} 2 2\n\ttwo\n{b} 5 3 1\nauthor Omar\nauthor-time 1700000500\nsummary plain change\nfilename f\n\tthree\n"
        );
        let m = parse_porcelain(&out);
        assert_eq!(m.len(), 3);
        assert_eq!((m[&2].author.as_str(), m[&2].time, m[&2].ticket.as_deref()), ("Jane Smith", 1_700_000_000, Some("ECOMD-4821")));
        assert_eq!((m[&3].author.as_str(), m[&3].summary.as_str(), m[&3].ticket.clone()), ("Omar", "plain change", None));
    }

    #[test]
    fn tickets_are_found_in_branches_and_messages() {
        assert_eq!(ticket("feature/ECOMD-5610-transpharm-quick-registration").as_deref(), Some("ECOMD-5610"));
        assert_eq!(ticket("ECOMD-4821: add loginDisabled flag").as_deref(), Some("ECOMD-4821"));
        assert_eq!(ticket("fix (JIRA-12)").as_deref(), Some("JIRA-12"));
        assert_eq!(ticket("UTF-8 handling"), None, "technical names are not tickets");
        assert_eq!(ticket("use SHA-256 here, see ECOMD-77").as_deref(), Some("ECOMD-77"));
        assert_eq!(ticket("refactor stuff"), None);
        assert_eq!(ticket("A-1"), None);
    }

    #[test]
    fn origins_for_commits_staged_and_unstaged_changes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let repo = Repository::init(p).unwrap();
        commit(&repo, p, "one\ntwo\nthree\nfour\n", "ABC-1 first");
        let c2 = commit(&repo, p, "one\nTWO\nthree\nfour\n", "ABC-2 shout two");

        // A commit diff's old side is its parent; staged changes' old side is HEAD.
        let at_head = origins(p, "f.txt", OldSide::Commit(c2)).unwrap();
        assert_eq!(origin_summary(&at_head, 1), "ABC-1 first");
        assert_eq!(origin_summary(&at_head, 2), "ABC-2 shout two");
        match at_head.lines.get(&2) {
            Some(Origin::Commit(c)) => assert_eq!(c.ticket.as_deref(), Some("ABC-2")),
            o => panic!("{o:?}"),
        }

        // Stage an inserted line at the top; the unstaged diff's old side is the index, whose
        // line numbers are shifted by one against HEAD.
        fs::write(p.join("f.txt"), "zero\none\nTWO\nthree\nfour\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("f.txt")).unwrap();
        idx.write().unwrap();
        let staged = origins(p, "f.txt", OldSide::Index).unwrap();
        assert_eq!(origin_summary(&staged, 1), "staged");
        assert_eq!(origin_summary(&staged, 2), "ABC-1 first");
        assert_eq!(origin_summary(&staged, 3), "ABC-2 shout two");
        assert_eq!(origin_summary(&staged, 5), "ABC-1 first");
        assert_eq!(staged.rev_range(3, 3), Some((2, 2)), "index line 3 is HEAD line 2");
        assert_eq!(staged.rev_range(1, 1), None, "a staged-only line has no history yet");
    }

    #[test]
    fn line_history_lists_the_commits_that_changed_those_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let repo = Repository::init(p).unwrap();
        commit(&repo, p, "a\nb\nc\n", "ABC-1 start");
        commit(&repo, p, "a\nB\nc\n", "ABC-2 capital b");
        let c3 = commit(&repo, p, "a\nB\nC\n", "ABC-3 capital c");
        let h = line_history(p, c3, "f.txt", 2, 2, 50).unwrap();
        let summaries: Vec<&str> = h.iter().map(|c| c.summary.as_str()).collect();
        assert_eq!(summaries, ["ABC-2 capital b", "ABC-1 start"], "line 3's change is not part of line 2's history");
        assert!(h[0].diff.iter().any(|l| l == "-b") && h[0].diff.iter().any(|l| l == "+B"), "{:?}", h[0].diff);
        assert_eq!(h[0].ticket.as_deref(), Some("ABC-2"));
    }
}

#[cfg(test)]
mod real_repo {
    /// Manual check against a real repository: GITGOING_REPO=/path GITGOING_FILE=rel/path
    #[test]
    #[ignore]
    fn real_repo_origins() {
        let (Ok(repo), Ok(file)) = (std::env::var("GITGOING_REPO"), std::env::var("GITGOING_FILE")) else { return };
        let t = std::time::Instant::now();
        let o = super::for_working_changes(std::path::Path::new(&repo), &file, true);
        println!("staged: {:?} in {:?}", o.as_ref().map(|o| (o.lines.len(), o.rev, o.context_ticket.clone())), t.elapsed());
        if let Ok(o) = &o {
            for n in 1..=o.lines.len().min(400) as u32 {
                if let Some(super::Origin::Commit(c)) = o.lines.get(&n) {
                    if n < 6 { println!("line {n}: {} | {} | {:?}", c.summary, c.author, c.ticket); }
                }
            }
        } else if let Err(e) = &o { println!("ERROR: {}", e.message()); }
    }
}
