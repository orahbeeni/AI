//! On Windows builds, embed the icon and version info into the .exe.

fn main() {
    println!("cargo:rerun-if-changed=windows/sketchframe.rc");
    println!("cargo:rerun-if-changed=windows/icon.ico");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "windows" || env != "gnu" {
        return;
    }
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let out = out_dir.join("sketchframe-res.o");
    // the version in the .exe's properties comes from Cargo.toml
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let mut parts: Vec<String> = version.split(['.', '-']).take(3).map(|p| p.parse::<u32>().unwrap_or(0).to_string()).collect();
    parts.resize(3, "0".into());
    let rc_text = std::fs::read_to_string("windows/sketchframe.rc")
        .unwrap_or_default()
        .replace("@VERSION4@", &format!("{},0", parts.join(",")))
        .replace("@VERSION@", &version);
    let rc = out_dir.join("sketchframe.rc");
    let _ = std::fs::write(&rc, rc_text);
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| {
        if cfg!(windows) {
            "windres".into()
        } else {
            "x86_64-w64-mingw32-windres".into()
        }
    });
    let status = std::process::Command::new(&windres)
        .current_dir("windows")
        .arg(&rc)
        .args(["-O", "coff", "-o"])
        .arg(&out)
        .status();
    match status {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg-bins={}", out.display()),
        _ => println!("cargo:warning=could not run {windres}; the .exe will have no icon"),
    }
}
