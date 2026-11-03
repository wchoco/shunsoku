//! 設定ファイルの場所の判定（ポータブルモード）。
//!
//! exe と同じフォルダに `settings.toml` があればそれを使い（ポータブルモード）、なければ
//! `%APPDATA%\shunsoku\settings.toml` を使う。ログ（`shunsoku.log`）は設定ファイルと同じフォルダに書く。
//! ファイルの有無はアプリが確かめ、結果を `exists` で渡す。

use std::path::{Path, PathBuf};

/// 設定ファイルの名前。
const FILE_NAME: &str = "settings.toml";

/// 使う設定ファイルの場所。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsLocation {
    /// 設定ファイルのパス。まだ無いこともある。
    pub settings_file: PathBuf,
    /// exe の横の設定ファイルを使うか（ポータブルモード）。
    pub portable: bool,
}

impl SettingsLocation {
    /// 設定ファイルを置くフォルダ。ログもここに書く。
    pub fn dir(&self) -> &Path {
        self.settings_file.parent().unwrap_or(Path::new(""))
    }
}

/// 設定ファイルの場所を決める。
///
/// - `exe_dir`: exe のあるフォルダ。分からなければ None。
/// - `appdata`: `%APPDATA%`。設定されていなければ None。
/// - `exists`: パスにファイルがあるか。
///
/// exe の横に `settings.toml` があればそれ（ポータブルモード）、なければ `%APPDATA%\shunsoku\settings.toml`。
/// どちらも決まらなければ None（既定の設定で動き、ログは書かない）。
pub fn locate(
    exe_dir: Option<&Path>,
    appdata: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> Option<SettingsLocation> {
    if let Some(dir) = exe_dir {
        let file = dir.join(FILE_NAME);
        if exists(&file) {
            return Some(SettingsLocation {
                settings_file: file,
                portable: true,
            });
        }
    }
    appdata.map(|dir| SettingsLocation {
        settings_file: dir.join(crate::APP_NAME).join(FILE_NAME),
        portable: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_side_file_makes_portable_mode() {
        let exe_dir = Path::new(r"D:\tools\shunsoku");
        let appdata = Path::new(r"C:\Users\u\AppData\Roaming");
        let located = locate(Some(exe_dir), Some(appdata), |p| {
            p == Path::new(r"D:\tools\shunsoku").join("settings.toml")
        })
        .unwrap();
        assert!(located.portable);
        assert_eq!(
            located.settings_file,
            Path::new(r"D:\tools\shunsoku").join("settings.toml")
        );
        assert_eq!(located.dir(), Path::new(r"D:\tools\shunsoku"));
    }

    #[test]
    fn without_exe_side_file_uses_appdata() {
        let exe_dir = Path::new(r"D:\tools\shunsoku");
        let appdata = Path::new(r"C:\Users\u\AppData\Roaming");
        let located = locate(Some(exe_dir), Some(appdata), |_| false).unwrap();
        assert!(!located.portable);
        let dir = Path::new(r"C:\Users\u\AppData\Roaming").join("shunsoku");
        assert_eq!(located.settings_file, dir.join("settings.toml"));
        assert_eq!(located.dir(), dir);
    }

    #[test]
    fn portable_mode_does_not_need_appdata() {
        let exe_dir = Path::new(r"D:\tools\shunsoku");
        let located = locate(Some(exe_dir), None, |_| true).unwrap();
        assert!(located.portable);
        assert_eq!(
            located.settings_file,
            Path::new(r"D:\tools\shunsoku").join("settings.toml")
        );
    }

    #[test]
    fn unknown_exe_dir_uses_appdata() {
        let appdata = Path::new(r"C:\Users\u\AppData\Roaming");
        let located = locate(None, Some(appdata), |_| true).unwrap();
        assert!(!located.portable);
        assert_eq!(
            located.settings_file,
            Path::new(r"C:\Users\u\AppData\Roaming")
                .join("shunsoku")
                .join("settings.toml")
        );
    }

    #[test]
    fn nothing_known_gives_no_location() {
        assert_eq!(locate(None, None, |_| true), None);
        assert_eq!(locate(Some(Path::new(r"D:\x")), None, |_| false), None);
    }
}
