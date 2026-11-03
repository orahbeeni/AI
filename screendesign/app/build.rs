//! On Windows builds, embed the icon and version info into the .exe.

fn main() {
    println!("cargo:rerun-if-changed=windows/sketchframe.rc");
    println!("cargo:rerun-if-changed=windows/icon.ico");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "windows" || env != "gnu" {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("sketchframe-res.o");
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| {
        if cfg!(windows) {
            "windres".into()
        } else {
            "x86_64-w64-mingw32-windres".into()
        }
    });
    let status = std::process::Command::new(&windres)
        .current_dir("windows")
        .args(["sketchframe.rc", "-O", "coff", "-o"])
        .arg(&out)
        .status();
    match status {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg-bins={}", out.display()),
        _ => println!("cargo:warning=could not run {windres}; the .exe will have no icon"),
    }
}
