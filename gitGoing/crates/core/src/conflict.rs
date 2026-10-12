//! Three-way merge model for the conflict editor.
//!
//! A conflicted file is split into regions: stretches that need no decision
//! (including changes that only one side made, which are merged automatically)
//! and real conflicts, where both sides changed the same lines differently.
//! A [`Resolution`] records which lines of each side the user picked for every
//! conflict and produces the resulting text.

use git2::Repository;
use similar::{Algorithm, DiffOp};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Region {
    /// Lines that are the same in the result no matter what.
    Common(Vec<String>),
    Conflict { ours: Vec<String>, base: Vec<String>, theirs: Vec<String> },
}

/// Which side of a conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
}

/// For each base line, the index of the line it matches in `other` (if any).
fn matches(base: &[String], other: &[String]) -> Vec<Option<usize>> {
    let mut m = vec![None; base.len()];
    for op in similar::capture_diff_slices(Algorithm::Myers, base, other) {
        if let DiffOp::Equal { old_index, new_index, len } = op {
            for k in 0..len {
                m[old_index + k] = Some(new_index + k);
            }
        }
    }
    m
}

fn push_common(out: &mut Vec<Region>, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(Region::Common(c)) => c.extend_from_slice(lines),
        _ => out.push(Region::Common(lines.to_vec())),
    }
}

/// Merge `ours` and `theirs` against their common ancestor `base`.
///
/// Base lines that both sides kept unchanged are anchors; between two anchors,
/// a change made by only one side (or made identically by both) is taken
/// automatically, and anything else is a conflict.
pub fn diff3(base: &[String], ours: &[String], theirs: &[String]) -> Vec<Region> {
    let mo = matches(base, ours);
    let mt = matches(base, theirs);
    let mut out = Vec::new();
    let (mut b, mut o, mut t) = (0usize, 0usize, 0usize);

    let chunk = |out: &mut Vec<Region>, bs: &[String], os: &[String], ts: &[String]| {
        if bs.is_empty() && os.is_empty() && ts.is_empty() {
            return;
        }
        if os == bs {
            push_common(out, ts);
        } else if ts == bs || os == ts {
            push_common(out, os);
        } else {
            out.push(Region::Conflict { ours: os.to_vec(), base: bs.to_vec(), theirs: ts.to_vec() });
        }
    };

    for s in 0..base.len() {
        let (Some(so), Some(st)) = (mo[s], mt[s]) else { continue };
        // Anchors must advance on all three sides to stay in order.
        if so < o || st < t {
            continue;
        }
        chunk(&mut out, &base[b..s], &ours[o..so], &theirs[t..st]);
        push_common(&mut out, std::slice::from_ref(&base[s]));
        (b, o, t) = (s + 1, so + 1, st + 1);
    }
    chunk(&mut out, &base[b..], &ours[o..], &theirs[t..]);
    out
}

/// The user's picks for one conflict: a flag per line of each side.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pick {
    pub ours: Vec<bool>,
    pub theirs: Vec<bool>,
    /// The user has made a decision about this conflict.
    pub touched: bool,
    /// The picked lines in the order they were chosen; the result follows it.
    pub order: Vec<(Side, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Common,
    Ours,
    Theirs,
    /// A conflict (by ordinal) that has not been decided yet.
    Unresolved(usize),
}

#[derive(Clone, Debug)]
pub struct Resolution {
    pub regions: Vec<Region>,
    /// One entry per `Region::Conflict`, in order.
    pub picks: Vec<Pick>,
}

impl Resolution {
    pub fn new(regions: Vec<Region>) -> Self {
        let picks = regions
            .iter()
            .filter_map(|r| match r {
                Region::Conflict { ours, theirs, .. } => Some(Pick {
                    ours: vec![false; ours.len()],
                    theirs: vec![false; theirs.len()],
                    touched: false,
                    order: Vec::new(),
                }),
                Region::Common(_) => None,
            })
            .collect();
        Resolution { regions, picks }
    }

    pub fn conflict_count(&self) -> usize {
        self.picks.len()
    }

    pub fn unresolved_count(&self) -> usize {
        self.picks.iter().filter(|p| !p.touched).count()
    }

    /// Select every line of the chosen sides in conflict `c` (`false` clears that
    /// side). Picked lines are ordered ours first, then theirs.
    pub fn set_conflict(&mut self, c: usize, ours: bool, theirs: bool) {
        let p = &mut self.picks[c];
        p.ours.iter_mut().for_each(|x| *x = ours);
        p.theirs.iter_mut().for_each(|x| *x = theirs);
        p.order.clear();
        if ours {
            p.order.extend((0..p.ours.len()).map(|k| (Side::Ours, k)));
        }
        if theirs {
            p.order.extend((0..p.theirs.len()).map(|k| (Side::Theirs, k)));
        }
        p.touched = true;
    }

    /// Take everything from one side (or from both) for every conflict.
    pub fn set_all(&mut self, ours: bool, theirs: bool) {
        for c in 0..self.picks.len() {
            self.set_conflict(c, ours, theirs);
        }
    }

    /// Select (or clear) all lines of one side of conflict `c`, leaving the
    /// other side's picks alone. Newly selected lines go after existing picks.
    pub fn set_side(&mut self, c: usize, side: Side, on: bool) {
        let p = &mut self.picks[c];
        let flags = match side {
            Side::Ours => &mut p.ours,
            Side::Theirs => &mut p.theirs,
        };
        flags.iter_mut().for_each(|x| *x = on);
        let n = flags.len();
        p.order.retain(|(s, _)| *s != side);
        if on {
            p.order.extend((0..n).map(|k| (side, k)));
        }
        p.touched = true;
    }

    /// Toggle one line. A line you pick now goes after the ones picked before it.
    pub fn toggle(&mut self, c: usize, side: Side, line: usize) {
        let p = &mut self.picks[c];
        let flags = match side {
            Side::Ours => &mut p.ours,
            Side::Theirs => &mut p.theirs,
        };
        if let Some(f) = flags.get_mut(line) {
            *f = !*f;
            if *f {
                p.order.push((side, line));
            } else {
                p.order.retain(|e| *e != (side, line));
            }
            p.touched = true;
        }
    }

    /// The merged text as lines. Within a conflict, lines appear in the order
    /// they were picked. Undecided conflicts appear as a single marker row.
    pub fn result(&self) -> Vec<(String, Origin)> {
        let mut out = Vec::new();
        let mut c = 0;
        for r in &self.regions {
            match r {
                Region::Common(lines) => out.extend(lines.iter().map(|l| (l.clone(), Origin::Common))),
                Region::Conflict { ours, theirs, .. } => {
                    let p = &self.picks[c];
                    if p.touched {
                        for (side, k) in &p.order {
                            let (lines, origin) = match side {
                                Side::Ours => (ours, Origin::Ours),
                                Side::Theirs => (theirs, Origin::Theirs),
                            };
                            if let Some(l) = lines.get(*k) {
                                out.push((l.clone(), origin));
                            }
                        }
                    } else {
                        out.push((format!("Unresolved conflict {}", c + 1), Origin::Unresolved(c)));
                    }
                    c += 1;
                }
            }
        }
        out
    }
}

/// Join lines back into file text using the file's own conventions.
pub fn to_text(lines: &[String], crlf: bool, final_newline: bool) -> String {
    let eol = if crlf { "\r\n" } else { "\n" };
    let mut s = lines.join(eol);
    if final_newline && !lines.is_empty() {
        s.push_str(eol);
    }
    s
}

fn split_lines(text: &str) -> (Vec<String>, bool, bool) {
    let crlf = text.contains("\r\n");
    let final_newline = text.ends_with('\n');
    let mut lines: Vec<String> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned()).collect();
    if final_newline {
        lines.pop();
    }
    if text.is_empty() {
        lines.clear();
    }
    (lines, crlf, final_newline)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Both sides changed the text of the file.
    Text,
    Binary,
    /// Deleted on our side, modified on theirs.
    DeletedByUs,
    /// Modified on our side, deleted on theirs.
    DeletedByThem,
}

/// Everything the conflict editor needs for one file.
#[derive(Clone, Debug)]
pub struct ConflictFile {
    pub path: String,
    pub kind: Kind,
    pub regions: Vec<Region>,
    pub ours_label: String,
    pub theirs_label: String,
    pub crlf: bool,
    pub final_newline: bool,
}

fn blob_text(repo: &Repository, entry: &Option<git2::IndexEntry>) -> Result<Option<Vec<u8>>, git2::Error> {
    entry.as_ref().map(|e| repo.find_blob(e.id).map(|b| b.content().to_vec())).transpose()
}

/// Describe the two sides: for a merge, the branch being merged in; for a
/// rebase the roles are reversed ("ours" is the branch being rebased onto).
fn labels(repo: &Repository) -> (String, String) {
    use git2::RepositoryState as S;
    let head = repo.head().ok().and_then(|h| h.shorthand().map(str::to_owned));
    match repo.state() {
        S::Rebase | S::RebaseInteractive | S::RebaseMerge => (
            "Upstream (rebasing onto)".to_owned(),
            "Your commit being applied".to_owned(),
        ),
        S::Merge => {
            let incoming = repo
                .message()
                .ok()
                .and_then(|m| {
                    let first = m.lines().next()?.to_owned();
                    let a = first.find('\'')?;
                    let b = first[a + 1..].find('\'')?;
                    Some(first[a + 1..a + 1 + b].to_owned())
                })
                .unwrap_or_else(|| "Incoming".to_owned());
            (head.unwrap_or_else(|| "Current".to_owned()), incoming)
        }
        _ => (head.unwrap_or_else(|| "Current".to_owned()), "Incoming".to_owned()),
    }
}

pub fn load(repo_path: &Path, path: &str) -> Result<ConflictFile, String> {
    let e = |e: git2::Error| e.message().to_owned();
    let repo = Repository::discover(repo_path).map_err(e)?;
    let index = repo.index().map_err(e)?;
    let conflict = index
        .conflicts()
        .map_err(e)?
        .filter_map(Result::ok)
        .find(|c| [&c.our, &c.their, &c.ancestor].into_iter().flatten().any(|x| x.path == path.as_bytes()))
        .ok_or_else(|| format!("'{path}' is not conflicted"))?;
    let (ours_label, theirs_label) = labels(&repo);
    let mut file = ConflictFile {
        path: path.to_owned(),
        kind: Kind::Text,
        regions: Vec::new(),
        ours_label,
        theirs_label,
        crlf: false,
        final_newline: true,
    };
    let (ours, theirs) = (blob_text(&repo, &conflict.our).map_err(e)?, blob_text(&repo, &conflict.their).map_err(e)?);
    let (Some(ours), Some(theirs)) = (ours, theirs) else {
        file.kind = if conflict.our.is_none() { Kind::DeletedByUs } else { Kind::DeletedByThem };
        return Ok(file);
    };
    let base = blob_text(&repo, &conflict.ancestor).map_err(e)?.unwrap_or_default();
    if [&ours, &theirs, &base].iter().any(|b| b.contains(&0)) {
        file.kind = Kind::Binary;
        return Ok(file);
    }
    let (o, o_crlf, o_nl) = split_lines(&String::from_utf8_lossy(&ours));
    let (t, _, _) = split_lines(&String::from_utf8_lossy(&theirs));
    let (b, _, _) = split_lines(&String::from_utf8_lossy(&base));
    file.crlf = o_crlf;
    file.final_newline = o_nl;
    file.regions = diff3(&b, &o, &t);
    Ok(file)
}

/// Write the resolved content into the working tree and mark the file resolved.
pub fn write_resolution(repo_path: &Path, path: &str, content: &str) -> Result<(), String> {
    let e = |e: git2::Error| e.message().to_owned();
    let repo = Repository::discover(repo_path).map_err(e)?;
    let workdir = repo.workdir().ok_or("repository has no working directory")?.to_path_buf();
    let p = Path::new(path);
    if p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("path is outside the working directory".into());
    }
    std::fs::write(workdir.join(p), content).map_err(|e| e.to_string())?;
    let mut index = repo.index().map_err(e)?;
    index.add_path(p).map_err(e)?; // replaces the conflict entries with a normal one
    index.write().map_err(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Vec<String> {
        if s.is_empty() { Vec::new() } else { s.split(' ').map(str::to_owned).collect() }
    }
    fn conflict(ours: &str, base: &str, theirs: &str) -> Region {
        Region::Conflict { ours: v(ours), base: v(base), theirs: v(theirs) }
    }

    #[test]
    fn non_overlapping_changes_merge_automatically() {
        // ours changes b, theirs changes e.
        let r = diff3(&v("a b c d e"), &v("a B c d e"), &v("a b c d E"));
        assert_eq!(r, vec![Region::Common(v("a B c d E"))]);
    }

    #[test]
    fn same_change_on_both_sides_is_not_a_conflict() {
        let r = diff3(&v("a b c"), &v("a X c"), &v("a X c"));
        assert_eq!(r, vec![Region::Common(v("a X c"))]);
    }

    #[test]
    fn overlapping_changes_conflict() {
        let r = diff3(&v("a b c d"), &v("a O c d"), &v("a T c d"));
        assert_eq!(r, vec![Region::Common(v("a")), conflict("O", "b", "T"), Region::Common(v("c d"))]);
    }

    #[test]
    fn modify_versus_delete_conflicts() {
        let r = diff3(&v("a b c"), &v("a b2 c"), &v("a c"));
        assert_eq!(r, vec![Region::Common(v("a")), conflict("b2", "b", ""), Region::Common(v("c"))]);
    }

    #[test]
    fn both_added_without_a_base() {
        let r = diff3(&[], &v("x y"), &v("p q r"));
        assert_eq!(r, vec![conflict("x y", "", "p q r")]);
        // Identical additions are not a conflict.
        assert_eq!(diff3(&[], &v("x"), &v("x")), vec![Region::Common(v("x"))]);
    }

    #[test]
    fn multiple_conflicts_and_empty_inputs() {
        let r = diff3(&v("1 2 3 4 5"), &v("1 A 3 B 5"), &v("1 X 3 Y 5"));
        assert_eq!(r.iter().filter(|x| matches!(x, Region::Conflict { .. })).count(), 2);
        assert!(diff3(&[], &[], &[]).is_empty());
    }

    #[test]
    fn resolution_picks_lines_and_orders_ours_first() {
        let regions = diff3(&v("a b c"), &v("a O1 O2 c"), &v("a T1 c"));
        let mut res = Resolution::new(regions);
        assert_eq!((res.conflict_count(), res.unresolved_count()), (1, 1));
        let r = res.result();
        assert!(matches!(r[1].1, Origin::Unresolved(0)), "undecided conflicts show a marker");

        res.set_conflict(0, true, true);
        let lines: Vec<_> = res.result().into_iter().map(|(l, _)| l).collect();
        assert_eq!(lines, ["a", "O1", "O2", "T1", "c"]);

        res.toggle(0, Side::Ours, 0); // drop O1
        res.toggle(0, Side::Theirs, 0); // drop T1
        let lines: Vec<_> = res.result().into_iter().map(|(l, _)| l).collect();
        assert_eq!(lines, ["a", "O2", "c"]);

        // Lines appear in the order they were clicked, across both sides.
        res.set_all(false, false);
        res.toggle(0, Side::Theirs, 0);
        res.toggle(0, Side::Ours, 0);
        res.toggle(0, Side::Ours, 1);
        let lines: Vec<_> = res.result().into_iter().map(|(l, _)| l).collect();
        assert_eq!(lines, ["a", "T1", "O1", "O2", "c"]);
        // Re-adding a removed line puts it last; taking a whole side appends it.
        res.toggle(0, Side::Ours, 0);
        res.toggle(0, Side::Ours, 0);
        let lines: Vec<_> = res.result().into_iter().map(|(l, _)| l).collect();
        assert_eq!(lines, ["a", "T1", "O2", "O1", "c"]);
        res.set_side(0, Side::Theirs, false);
        res.set_side(0, Side::Theirs, true);
        let lines: Vec<_> = res.result().into_iter().map(|(l, _)| l).collect();
        assert_eq!(lines, ["a", "O2", "O1", "T1", "c"]);

        res.set_all(false, true);
        let lines: Vec<_> = res.result().into_iter().map(|(l, _)| l).collect();
        assert_eq!(lines, ["a", "T1", "c"]);
        assert_eq!(res.unresolved_count(), 0);
    }

    #[test]
    fn text_conventions_are_preserved() {
        let (lines, crlf, nl) = split_lines("a\r\nb\r\n");
        assert_eq!((lines.clone(), crlf, nl), (vec!["a".to_owned(), "b".to_owned()], true, true));
        assert_eq!(to_text(&lines, crlf, nl), "a\r\nb\r\n");
        let (lines, crlf, nl) = split_lines("a\nb");
        assert_eq!(to_text(&lines, crlf, nl), "a\nb");
        assert!(split_lines("").0.is_empty());
    }

    #[test]
    fn loads_a_real_conflict_and_saves_a_resolution() {
        use crate::{branches, status};
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let repo = Repository::init(p).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        let w = |c: &str, m: &str| {
            std::fs::write(p.join("f"), c).unwrap();
            status::stage(p, &["f".into()]).unwrap();
            status::commit(p, m).unwrap();
        };
        w("one\ntwo\nthree\nfour\nfive\n", "base");
        let main = repo.head().unwrap().shorthand().unwrap().to_owned();
        branches::create_and_checkout(p, "feature").unwrap();
        w("one\nTWO-feature\nthree\nfour\nFIVE-feature\n", "feature");
        branches::checkout(p, &main).unwrap();
        w("one\nTWO-main\nthree\nfour\nfive\n", "main");
        crate::merge::merge(p, "feature").unwrap();

        let f = load(p, "f").unwrap();
        assert_eq!(f.kind, Kind::Text);
        assert_eq!(f.theirs_label, "feature");
        let mut res = Resolution::new(f.regions.clone());
        assert_eq!(res.conflict_count(), 1, "the FIVE change merges automatically");
        res.set_all(false, true);
        let text = to_text(&res.result().into_iter().map(|(l, _)| l).collect::<Vec<_>>(), f.crlf, f.final_newline);
        assert_eq!(text, "one\nTWO-feature\nthree\nfour\nFIVE-feature\n");

        write_resolution(p, "f", &text).unwrap();
        assert!(status::status(p).unwrap().unstaged.iter().all(|e| e.path != "f"), "no longer conflicted");
        status::commit(p, "Merge feature").unwrap();
    }
}
