//! Times a full search of the home folder for a file that doesn't exist (the worst case).
use organise_core::scan::{search, Target};
use std::path::PathBuf;
fn main() {
    let home = PathBuf::from(std::env::var("HOME").unwrap());
    let t = Target { id: "x".into(), name: "no-such-file-anywhere.xyz".into(), is_dir: false, size: Some(1), inode: Some(1), dev: Some(1),
        fingerprint: None, origin: Some(home.join("Documents/old/no-such-file-anywhere.xyz")) };
    let start = std::time::Instant::now();
    let r = search(&[t], &[home], true, &std::sync::atomic::AtomicBool::new(false));
    println!("{:?} in {:.1}s", r.get("x"), start.elapsed().as_secs_f64());
}
