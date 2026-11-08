//! Embeds assets/icon/meetings.ico in sidenote.exe as its Windows icon (resource ID 1, the
//! one Explorer, the taskbar and the Start menu use). Calls the GNU toolchain's windres, which
//! build.ps1 puts on PATH next to `as` and `gcc`.
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon/meetings.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        println!("cargo:warning=app icon is only embedded for the windows-gnu target");
        return;
    }
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let ico = manifest.join("assets/icon/meetings.ico");
    let rc = out.join("icon.rc");
    let obj = out.join("icon.o");
    let ico_path = ico.to_str().unwrap().replace('\\', "/");
    std::fs::write(&rc, format!("1 ICON \"{ico_path}\"\n")).unwrap();
    let status = Command::new("windres")
        .args(["--input"])
        .arg(&rc)
        .args(["--output-format=coff", "--output"])
        .arg(&obj)
        .status()
        .unwrap_or_else(|e| panic!("windres not found on PATH ({e}); run ./build.ps1"));
    assert!(status.success(), "windres failed on {}", rc.display());
    println!("cargo:rustc-link-arg-bins={}", obj.display());
}
