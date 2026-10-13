//! Comparing two branches: which files differ, narrowed down by partial file names.

use crate::diff::{kind_of, ChangeKind};
use git2::{DiffOptions, Repository};
use std::path::Path;

/// A file that differs between two refs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changed {
    pub path: String,
    pub kind: ChangeKind,
}

/// Every file whose content differs between `base` and `other` (full ref names
/// or anything `git rev-parse` understands), sorted by path. This is a plain
/// tree-to-tree comparison, like `git diff base other --name-status`.
pub fn changed_files(repo_path: &Path, base: &str, other: &str) -> Result<Vec<Changed>, git2::Error> {
    crate::gitlog::read(repo_path, format!("git diff --name-status {} {}", crate::gitlog::quote(base), crate::gitlog::quote(other)));
    let repo = Repository::discover(repo_path)?;
    let tree = |name: &str| repo.revparse_single(name)?.peel_to_tree();
    let (a, b) = (tree(base)?, tree(other)?);
    let mut opts = DiffOptions::new();
    opts.include_typechange(true);
    let diff = repo.diff_tree_to_tree(Some(&a), Some(&b), Some(&mut opts))?;
    let mut out: Vec<Changed> = diff
        .deltas()
        .filter_map(|d| {
            let path = d.new_file().path().or_else(|| d.old_file().path())?.to_string_lossy().into_owned();
            Some(Changed { path, kind: kind_of(d.status()) })
        })
        .collect();
    out.sort_by(|x, y| x.path.cmp(&y.path));
    Ok(out)
}

/// Split a comma separated list of partial file names. Blank entries are dropped.
pub fn parse_terms(input: &str) -> Vec<String> {
    input.split(',').map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect()
}

/// Whether `path` matches any of `terms` (already lower-case) as a partial name.
/// A term is looked for in the file name; a term containing `/` is looked for in
/// the whole path, so `src/user` works too. No terms matches everything.
pub fn matches(path: &str, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let path = path.to_lowercase();
    let name = path.rsplit('/').next().unwrap_or(&path);
    terms.iter().any(|t| if t.contains('/') { path.contains(t.as_str()) } else { name.contains(t.as_str()) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?} failed");
    }

    #[test]
    fn terms_are_trimmed_lowercased_and_blanks_dropped() {
        assert_eq!(parse_terms(" User , ORDER,, "), ["user", "order"]);
        assert!(parse_terms(" , ").is_empty());
    }

    #[test]
    fn terms_match_part_of_the_file_name_in_any_case() {
        let t = parse_terms("user, ord");
        assert!(matches("src/UserService.java", &t));
        assert!(matches("a/b/order_dao.rs", &t));
        assert!(!matches("src/account.rs", &t));
        // A directory name alone doesn't match a plain term...
        assert!(!matches("users/readme.md", &t));
        // ...but a term with a slash is looked for in the whole path.
        assert!(matches("users/readme.md", &parse_terms("users/read")));
        // No terms: everything.
        assert!(matches("anything", &[]));
    }

    #[test]
    fn lists_the_files_that_differ_between_two_branches() {
        if Command::new("git").arg("--version").output().is_err() {
            return; // git not installed
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("same.txt"), "same").unwrap();
        std::fs::write(p.join("edit.txt"), "one").unwrap();
        std::fs::write(p.join("gone.txt"), "x").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "base"]);
        git(p, &["checkout", "-q", "-b", "other"]);
        std::fs::write(p.join("edit.txt"), "two").unwrap();
        std::fs::write(p.join("new.txt"), "n").unwrap();
        std::fs::remove_file(p.join("gone.txt")).unwrap();
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", "change"]);

        let got = changed_files(p, "refs/heads/main", "refs/heads/other").unwrap();
        let got: Vec<_> = got.iter().map(|c| (c.path.as_str(), c.kind)).collect();
        assert_eq!(got, [("edit.txt", ChangeKind::Modified), ("gone.txt", ChangeKind::Deleted), ("new.txt", ChangeKind::Added)]);
        assert!(changed_files(p, "refs/heads/main", "refs/heads/nope").is_err());
    }
}
