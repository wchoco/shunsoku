//! ポータブルモードの設定ファイルの書き込み先（docs/manual-checks.md の M5-15、M5-16）。
//!
//! exe のフォルダと `%APPDATA%` は `%TEMP%` の下に作った試験用のフォルダを渡す。ユーザーの `%APPDATA%` と
//! 環境変数には触れない。

use std::path::{Path, PathBuf};
use std::process::Command;

use shunsoku_core::settings::{self, OwnWrites, Settings, WriteFailure};

use super::{locate_settings_at, write_settings_file};

/// `%TEMP%` の下の試験用のフォルダ。落ちても消す。
struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Sandbox {
        let root =
            std::env::temp_dir().join(format!("shunsoku-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("exe")).unwrap();
        std::fs::create_dir_all(root.join("appdata")).unwrap();
        Sandbox(root)
    }

    fn exe_dir(&self) -> PathBuf {
        self.0.join("exe")
    }

    fn appdata(&self) -> PathBuf {
        self.0.join("appdata")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // 書き込みを断る ACE を足していれば外してから消す
        let _ = Command::new("icacls")
            .arg(self.exe_dir())
            .args(["/remove:d", "*S-1-1-0"])
            .output();
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).unwrap().next().is_none()
}

fn changed_settings() -> Settings {
    let mut new = Settings::default();
    new.general.trash_limit = 3;
    new
}

#[test]
fn an_empty_settings_file_next_to_the_exe_is_written_and_appdata_is_not_used() {
    let sandbox = Sandbox::new("portable");
    let file = sandbox.exe_dir().join("settings.toml");
    std::fs::write(&file, "").unwrap();

    let location = locate_settings_at(Some(&sandbox.exe_dir()), Some(&sandbox.appdata())).unwrap();
    assert!(location.portable);
    assert_eq!(location.settings_file, file);
    // ログは設定ファイルと同じフォルダに書く（`run` が `logger::init(location.dir())` に渡す）
    assert_eq!(location.dir(), sandbox.exe_dir());

    write_settings_file(
        &location.settings_file,
        &changed_settings(),
        &mut OwnWrites::default(),
    )
    .unwrap();

    let written = std::fs::read_to_string(&file).unwrap();
    let (read_back, _) = settings::load(&written);
    assert_eq!(read_back.general.trash_limit, 3);
    assert!(is_empty(&sandbox.appdata()), "%APPDATA% に何か書かれた");
}

#[test]
fn a_read_only_portable_folder_fails_without_falling_back_to_appdata() {
    let sandbox = Sandbox::new("read-only");
    let file = sandbox.exe_dir().join("settings.toml");
    std::fs::write(&file, "").unwrap();
    // Everyone に、フォルダへのファイルの作成と、ファイルへの書き込みを断る。フォルダの読み取り専用の属性は、
    // Windows ではファイルの作成を止めない。`(W)` で断ると SYNCHRONIZE も断られ、ファイルを開くこと自体が
    // できなくなる（ファイルがないと判定される）ので、権利を個別に挙げる
    let denied = Command::new("icacls")
        .arg(sandbox.exe_dir())
        .args(["/deny", "*S-1-1-0:(OI)(CI)(WD,AD)"])
        .output()
        .unwrap();
    assert!(denied.status.success(), "{denied:?}");

    let location = locate_settings_at(Some(&sandbox.exe_dir()), Some(&sandbox.appdata())).unwrap();
    assert!(location.portable);
    let result = write_settings_file(
        &location.settings_file,
        &changed_settings(),
        &mut OwnWrites::default(),
    );

    assert!(matches!(result, Err(WriteFailure::Io { .. })), "{result:?}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    assert!(is_empty(&sandbox.appdata()), "%APPDATA% に何か書かれた");
}
