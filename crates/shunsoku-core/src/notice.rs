//! トレイの通知を出すかと、通知のクリックで開くフォルダ。
//!
//! - 新しい通知は前の通知を置き換える。通知のクリックは中身を持たないので、出ている通知のフォルダを覚えておく。
//! - 設定で通知をオフにしても、止めるのは成功の通知だけ（エラーと警告は出す）。

use std::path::{Path, PathBuf};

/// 出ている通知と、そのクリックで開くフォルダ。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notices {
    folder: Option<PathBuf>,
}

impl Notices {
    /// 通知を出す前に呼ぶ。`success` は成功の通知か、`enabled` は設定で通知をオンにしているか。出すなら true。
    ///
    /// 出すなら、置き換わる前の通知のフォルダを忘れる。出さないなら、前の通知は出たままなので、そのフォルダも
    /// 覚えたままにする（残った通知をクリックしたら、その通知のフォルダを開く）。
    pub fn showing(&mut self, success: bool, enabled: bool) -> bool {
        let show = enabled || !success;
        if show {
            self.folder = None;
        }
        show
    }

    /// 出した通知のクリックで `folder` を開くようにする。
    pub fn opens(&mut self, folder: PathBuf) {
        self.folder = Some(folder);
    }

    /// 通知がクリックされた: 開くフォルダ。
    pub fn clicked(&self) -> Option<&Path> {
        self.folder.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved_to(folder: &str) -> Notices {
        let mut notices = Notices::default();
        assert!(notices.showing(true, true));
        notices.opens(PathBuf::from(folder));
        notices
    }

    #[test]
    fn clicking_a_save_notice_opens_its_folder() {
        let notices = saved_to(r"C:\shots");
        assert_eq!(notices.clicked(), Some(Path::new(r"C:\shots")));
    }

    #[test]
    fn a_new_notice_replaces_the_old_one_and_its_folder() {
        let mut notices = saved_to(r"C:\shots");
        assert!(notices.showing(false, true));
        assert_eq!(notices.clicked(), None);

        let mut notices = saved_to(r"C:\shots");
        assert!(notices.showing(true, true));
        notices.opens(PathBuf::from(r"D:\other"));
        assert_eq!(notices.clicked(), Some(Path::new(r"D:\other")));
    }

    #[test]
    fn a_success_notice_held_back_by_the_setting_leaves_the_old_notice_and_its_folder() {
        let mut notices = saved_to(r"C:\shots");
        assert!(!notices.showing(true, false));
        assert_eq!(notices.clicked(), Some(Path::new(r"C:\shots")));
    }

    #[test]
    fn errors_and_warnings_show_even_with_notifications_off() {
        let mut notices = saved_to(r"C:\shots");
        assert!(notices.showing(false, false));
        assert_eq!(notices.clicked(), None);
    }
}
