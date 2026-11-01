//! アプリケーション マニフェスト（Per-Monitor V2、Common Controls v6 など）を MSVC のリンカで埋め込む。
//! build-dependencies は使わない（ADR-0005 の依存の制限）。

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    // build.rs はホストで動くので、cfg! ではなく対象の OS と環境を環境変数で見る
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os == "windows" && env == "msvc" {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        let path = std::path::Path::new(&dir).join("app.manifest");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            path.display()
        );
        // trustInfo は app.manifest に書いてある
        println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:NO");
    }
}
