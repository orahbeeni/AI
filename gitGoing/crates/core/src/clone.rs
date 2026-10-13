//! Cloning and creating repositories.

use crate::gitcmd;
use std::path::Path;

/// Guess a folder name from a clone URL: `https://host/o/repo.git` → `repo`.
pub fn repo_name_from_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let last = trimmed.rsplit(['/', ':', '\\']).next().unwrap_or("");
    last.strip_suffix(".git").unwrap_or(last).to_owned()
}

/// `git clone <url> <dest>`. `dest` must not exist yet (or be an empty folder).
pub fn clone(url: &str, dest: &Path) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("enter a repository URL".into());
    }
    if dest.exists() && dest.read_dir().map_err(|e| e.to_string())?.next().is_some() {
        return Err(format!("{} already exists and is not empty", dest.display()));
    }
    let dest_str = dest.to_string_lossy();
    gitcmd::run(None, &["clone", "--", url, &dest_str]).map(|_| ())
}

/// Create a repository in `path` (creating the folder if needed).
pub fn init(path: &Path) -> Result<(), git2::Error> {
    crate::gitlog::equiv(path, format!("git init {}", crate::gitlog::quote(&path.to_string_lossy())));
    std::fs::create_dir_all(path).map_err(|e| git2::Error::from_str(&e.to_string()))?;
    git2::Repository::init(path).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_urls() {
        assert_eq!(repo_name_from_url("https://github.com/o/repo.git"), "repo");
        assert_eq!(repo_name_from_url("git@github.com:o/my-repo.git"), "my-repo");
        assert_eq!(repo_name_from_url("https://host/o/repo/"), "repo");
        assert_eq!(repo_name_from_url("/local/path/proj"), "proj");
        assert_eq!(repo_name_from_url(""), "");
    }

    #[test]
    fn init_and_clone_local() {
        if gitcmd::run(None, &["--version"]).is_err() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        init(&src).unwrap();
        assert!(crate::is_repo(&src));
        // Cloning an empty repository works and creates a repo.
        let dest = tmp.path().join("copy");
        clone(src.to_str().unwrap(), &dest).unwrap();
        assert!(crate::is_repo(&dest));
        // A non-empty destination is refused.
        let busy = tmp.path().join("busy");
        std::fs::create_dir(&busy).unwrap();
        std::fs::write(busy.join("keep"), "x").unwrap();
        assert!(clone(src.to_str().unwrap(), &busy).is_err());
        assert!(clone("", &dest).is_err());
    }
}
