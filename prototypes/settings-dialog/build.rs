// Compile settings.rc with rc.exe from the Windows SDK and link the .res,
// and embed app.manifest (Per-Monitor V2 + Common Controls v6) with the MSVC linker.
// No build-dependencies. Set RC=<path to rc.exe> to override the SDK lookup.
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=settings.rc");
    println!("cargo:rerun-if-env-changed=RC");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "windows" || env != "msvc" {
        return;
    }
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let manifest = dir.join("app.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
        manifest.display()
    );
    println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:NO");

    // rc.exe only exists on a Windows host. When type-checking from Linux
    // (cargo clippy --target x86_64-pc-windows-msvc) skip it.
    if !cfg!(windows) {
        println!("cargo:warning=host is not Windows: settings.rc is not compiled");
        return;
    }
    let rc = find_rc().expect("rc.exe not found (install the Windows SDK or set RC)");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("settings.res");
    let status = Command::new(&rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&out)
        .arg(dir.join("settings.rc"))
        .status()
        .expect("failed to run rc.exe");
    assert!(status.success(), "rc.exe failed: {status}");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

fn find_rc() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("RC") {
        return Some(PathBuf::from(p));
    }
    let root = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    // Newest versioned dir first (10.0.22000.0 > 10.0.17134.0 ...), then the unversioned one.
    let mut versions: Vec<(Vec<u32>, PathBuf)> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let v: Option<Vec<u32>> = name.split('.').map(|s| s.parse().ok()).collect();
            v.map(|v| (v, e.path()))
        })
        .collect();
    versions.sort();
    versions
        .into_iter()
        .rev()
        .map(|(_, p)| p.join(r"x64\rc.exe"))
        .chain([root.join(r"x64\rc.exe")])
        .find(|p| p.is_file())
}
