//! The user's global git configuration (commit author identity).

use std::path::PathBuf;

fn global_config_path() -> PathBuf {
    git2::Config::find_global().unwrap_or_else(|_| {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).unwrap_or_default();
        PathBuf::from(home).join(".gitconfig")
    })
}

/// `(user.name, user.email)` from the global git config (empty if unset).
pub fn read_identity() -> (String, String) {
    read_identity_from(&global_config_path())
}

pub fn write_identity(name: &str, email: &str) -> Result<(), String> {
    write_identity_to(&global_config_path(), name, email)
}

fn read_identity_from(path: &std::path::Path) -> (String, String) {
    let cfg = git2::Config::open(path);
    let get = |k: &str| cfg.as_ref().ok().and_then(|c| c.get_string(k).ok()).unwrap_or_default();
    (get("user.name"), get("user.email"))
}

fn write_identity_to(path: &std::path::Path, name: &str, email: &str) -> Result<(), String> {
    let e = |e: git2::Error| e.message().to_owned();
    let mut cfg = git2::Config::open(path).map_err(e)?;
    cfg.set_str("user.name", name.trim()).map_err(e)?;
    cfg.set_str("user.email", email.trim()).map_err(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".gitconfig");
        assert_eq!(read_identity_from(&file), (String::new(), String::new()));
        write_identity_to(&file, " Ada ", "ada@example.com").unwrap();
        assert_eq!(read_identity_from(&file), ("Ada".to_owned(), "ada@example.com".to_owned()));
    }
}
