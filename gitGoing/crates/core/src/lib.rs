//! UI-independent git logic for gitgoing.

pub mod branches;
pub mod clone;
pub mod commit_ops;
pub mod config;
pub mod conflict;
pub mod gitcmd;
pub mod gitlog;
pub mod diff;
pub mod graph;
pub mod history;
pub mod merge;
pub mod rebase;
pub mod remote;
pub mod remotes;
pub mod stash;
pub mod status;
pub mod system;
pub mod undo;

pub use git2::Oid;
pub use git2::Error as GitError;

/// Whether `path` is inside a git repository.
pub fn is_repo(path: &std::path::Path) -> bool {
    git2::Repository::discover(path).is_ok()
}
