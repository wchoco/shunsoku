//! 設定 GUI の適用での 3 者の突き合わせ（`merge`）。
//!
//! - base（開いた時点か最後に適用した時点の設定）、ours（GUI の写し）、theirs（適用の時点でファイルから読んだ
//!   設定）を項目（`Field`）ごとに比べる。GUI で変えた項目（base と ours が違う）だけを theirs に当て、外で
//!   変えた項目は theirs のまま残す。
//! - 外と GUI の両方で同じ項目を違う値に変えていたら衝突（`Merged::conflicts`）。`Merged::settings` は衝突を
//!   GUI の値にしたもの（「GUI の値で上書き」）で、衝突があれば書く前に確かめる。
//! - 項目は値の 1 つずつ（`[shortcuts]` と `[mouse]` は操作の id ごと）。`Settings` に項目を足したら、`merge`
//!   の分解（`..` なし）がコンパイルを止めるので、ここに 1 行足す。
//! - 定型領域は今は `Field::Regions` 1 つにまとめる（#98 で領域ごと・項目ごとにする）。

use std::collections::BTreeMap;

use super::{
    AnnotationDefaults, Border, General, Hotkeys, ScrapAppearance, SelectionScreen, Settings,
};

/// 突き合わせる項目。`Display` は設定ファイルのキー（`general.save_dir`、`shortcuts.copy` など）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    /// 名前の決まった値。設定ファイルのキーを `.` でつないだもの。
    Key(&'static str),
    /// `[shortcuts]` のスクラップ操作の id。
    Shortcut(String),
    /// `[mouse]` のスクラップ操作の id。
    Mouse(String),
    /// `[[regions]]` 全体。
    Regions,
}

impl std::fmt::Display for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Field::Key(key) => f.write_str(key),
            Field::Shortcut(id) => write!(f, "shortcuts.{id}"),
            Field::Mouse(id) => write!(f, "mouse.{id}"),
            Field::Regions => f.write_str("regions"),
        }
    }
}

/// 突き合わせの結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    /// 書く設定。衝突した項目は GUI の値にしてある。
    pub settings: Settings,
    /// 外と GUI の両方で違う値に変えた項目。空でなければ、書く前に「GUI の値で上書き」か「適用をやめる」かを
    /// 確かめる。
    pub conflicts: Vec<Field>,
}

impl Merged {
    /// 書く前に確かめる必要があるか。
    pub fn needs_confirmation(&self) -> bool {
        !self.conflicts.is_empty()
    }
}

/// base（`base`）から GUI で変えた差（`ours`）を、ファイルから読んだ設定（`theirs`）に当てる。
pub fn merge(base: &Settings, ours: &Settings, theirs: &Settings) -> Merged {
    let mut out = theirs.clone();
    let mut conflicts = Vec::new();
    let mut m = |field: Field, changed: bool, both: bool| {
        if changed && both {
            conflicts.push(field);
        }
    };

    // 項目を足したらここで止まる（`..` を使わない）。分解した値はどれも下で使う
    let Settings {
        general,
        hotkeys,
        shortcuts,
        mouse,
        scrap,
        annotation,
        selection,
        log_level,
        regions,
    } = ours;
    let General {
        save_dir,
        file_name_template,
        notifications,
        trash_limit,
        dpi_scaling,
        capture_cursor,
    } = general;
    let Hotkeys {
        select_capture,
        restore_last,
        active_window_capture,
    } = hotkeys;
    let ScrapAppearance {
        opacity,
        border,
        shadow,
    } = scrap;
    let Border {
        visible: border_visible,
        width: border_width,
        color: border_color,
    } = border;
    let AnnotationDefaults {
        color: annotation_color,
        width: annotation_width,
        font_size,
        font,
    } = annotation;
    let SelectionScreen { mode, crosshair } = selection;

    macro_rules! value {
        ($key:literal, $ours:expr, $($path:ident).+) => {{
            let (changed, both) = take(&base.$($path).+, $ours, &mut out.$($path).+);
            m(Field::Key($key), changed, both);
        }};
    }
    value!("general.save_dir", save_dir, general.save_dir);
    value!(
        "general.file_name_template",
        file_name_template,
        general.file_name_template
    );
    value!(
        "general.notifications",
        notifications,
        general.notifications
    );
    value!("general.trash_limit", trash_limit, general.trash_limit);
    value!("general.dpi_scaling", dpi_scaling, general.dpi_scaling);
    value!(
        "general.capture_cursor",
        capture_cursor,
        general.capture_cursor
    );
    value!(
        "hotkeys.select_capture",
        select_capture,
        hotkeys.select_capture
    );
    value!("hotkeys.restore_last", restore_last, hotkeys.restore_last);
    value!(
        "hotkeys.active_window_capture",
        active_window_capture,
        hotkeys.active_window_capture
    );
    value!("scrap.opacity", opacity, scrap.opacity);
    value!("scrap.border", border_visible, scrap.border.visible);
    value!("scrap.border_width", border_width, scrap.border.width);
    value!("scrap.border_color", border_color, scrap.border.color);
    value!("scrap.shadow", shadow, scrap.shadow);
    value!("annotation.color", annotation_color, annotation.color);
    value!("annotation.width", annotation_width, annotation.width);
    value!("annotation.font_size", font_size, annotation.font_size);
    value!("annotation.font", font, annotation.font);
    value!("selection.mode", mode, selection.mode);
    value!("selection.crosshair", crosshair, selection.crosshair);
    value!("log_level", log_level, log_level);
    for (id, changed, both) in take_map(&base.shortcuts, shortcuts, &mut out.shortcuts) {
        m(Field::Shortcut(id), changed, both);
    }
    for (id, changed, both) in take_map(&base.mouse, mouse, &mut out.mouse) {
        m(Field::Mouse(id), changed, both);
    }
    let (changed, both) = take(&base.regions, regions, &mut out.regions);
    m(Field::Regions, changed, both);

    Merged {
        settings: out,
        conflicts,
    }
}

/// 1 つの項目を突き合わせる。`out` は theirs の値で、GUI で変えていれば GUI の値にする。
/// （GUI で変えたか, 外でも違う値に変えていたか）を返す。
fn take<T: PartialEq + Clone>(base: &T, ours: &T, out: &mut T) -> (bool, bool) {
    if ours == base {
        return (false, false);
    }
    let both = *out != *base && *out != *ours;
    *out = ours.clone();
    (true, both)
}

/// id ごとの上書きの表を突き合わせる。表にないことも 1 つの値として比べる。
fn take_map<V: PartialEq + Clone>(
    base: &BTreeMap<String, V>,
    ours: &BTreeMap<String, V>,
    out: &mut BTreeMap<String, V>,
) -> Vec<(String, bool, bool)> {
    let ids: std::collections::BTreeSet<String> = base.keys().chain(ours.keys()).cloned().collect();
    ids.into_iter()
        .map(|id| {
            let mut value = out.get(&id).cloned();
            let (changed, both) =
                take(&base.get(&id).cloned(), &ours.get(&id).cloned(), &mut value);
            match value {
                Some(v) => out.insert(id.clone(), v),
                None => out.remove(&id),
            };
            (id, changed, both)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_only_changes_are_written_external_only_changes_stay_and_both_sides_conflict() {
        let base = Settings::default();
        let mut ours = base.clone();
        ours.general.trash_limit = 20; // GUI だけ
        ours.scrap.opacity = 50; // 両方
        let mut theirs = base.clone();
        theirs.general.notifications = false; // 外だけ
        theirs.scrap.opacity = 70; // 両方

        let merged = merge(&base, &ours, &theirs);

        assert_eq!(merged.settings.general.trash_limit, 20);
        assert!(!merged.settings.general.notifications);
        assert_eq!(merged.conflicts, vec![Field::Key("scrap.opacity")]);
    }

    #[test]
    fn without_conflicts_no_confirmation_is_needed() {
        let base = Settings::default();
        let mut ours = base.clone();
        ours.general.trash_limit = 20;
        let mut theirs = base.clone();
        theirs.general.notifications = false;
        // 両方で同じ値にしたのは衝突ではない
        ours.scrap.shadow = false;
        theirs.scrap.shadow = false;

        let merged = merge(&base, &ours, &theirs);

        assert!(!merged.needs_confirmation());
        let mut expected = base.clone();
        expected.general.trash_limit = 20;
        expected.general.notifications = false;
        expected.scrap.shadow = false;
        assert_eq!(merged.settings, expected);
    }

    #[test]
    fn overwriting_a_conflict_takes_the_gui_value() {
        let base = Settings::default();
        let mut ours = base.clone();
        ours.general.file_name_template = "gui_{date}".to_owned();
        let mut theirs = base.clone();
        theirs.general.file_name_template = "file_{date}".to_owned();

        let merged = merge(&base, &ours, &theirs);

        assert!(merged.needs_confirmation());
        assert_eq!(
            merged.conflicts,
            vec![Field::Key("general.file_name_template")]
        );
        assert_eq!(merged.settings.general.file_name_template, "gui_{date}");
    }

    #[test]
    fn shortcut_and_mouse_overrides_are_merged_per_operation() {
        use crate::scrap::ops::MouseGesture;
        let chord = |s: &str| s.parse::<crate::keys::Chord>().unwrap();
        let mut base = Settings::default();
        base.shortcuts
            .insert("copy".into(), Some(chord("Ctrl+Shift+C")));
        let mut ours = base.clone();
        ours.shortcuts.insert("close".into(), Some(chord("Ctrl+W"))); // GUI で足す
        ours.shortcuts.insert("save".into(), Some(chord("Ctrl+S"))); // 両方で違う値
        ours.mouse.insert("close".into(), None); // GUI で割り当てなしにする
        let mut theirs = base.clone();
        theirs.shortcuts.remove("copy"); // 外で消す
        theirs
            .shortcuts
            .insert("save".into(), Some(chord("Ctrl+Alt+S")));
        theirs
            .mouse
            .insert("toggle_compact".into(), Some(MouseGesture::MiddleClick));

        let merged = merge(&base, &ours, &theirs);

        assert_eq!(merged.conflicts, vec![Field::Shortcut("save".into())]);
        assert_eq!(merged.conflicts[0].to_string(), "shortcuts.save");
        let s = &merged.settings;
        assert_eq!(s.shortcuts.get("copy"), None);
        assert_eq!(s.shortcuts.get("close"), Some(&Some(chord("Ctrl+W"))));
        assert_eq!(s.shortcuts.get("save"), Some(&Some(chord("Ctrl+S"))));
        assert_eq!(s.mouse.get("close"), Some(&None));
        assert_eq!(
            s.mouse.get("toggle_compact"),
            Some(&Some(MouseGesture::MiddleClick))
        );
    }

    /// 項目のキーと、その項目だけを変える関数。
    type Change = (&'static str, fn(&mut Settings));

    /// 1 つの項目だけを既定値から変えるもの。`Settings` に項目を足したら、ここにも足す（`merge` の分解は
    /// コンパイルで止まるが、この一覧は止まらない）。
    fn one_field_changes() -> Vec<Change> {
        use crate::selection::Mode;
        use crate::settings::{DpiScaling, LogLevel, Region, Rgb};
        vec![
            ("general.save_dir", |s| {
                s.general.save_dir = Some("D:\\shots".into())
            }),
            ("general.file_name_template", |s| {
                s.general.file_name_template = "x_{date}".into()
            }),
            ("general.notifications", |s| s.general.notifications = false),
            ("general.trash_limit", |s| s.general.trash_limit = 3),
            ("general.dpi_scaling", |s| {
                s.general.dpi_scaling = DpiScaling::KeepApparentSize
            }),
            ("general.capture_cursor", |s| {
                s.general.capture_cursor = true
            }),
            ("hotkeys.select_capture", |s| {
                s.hotkeys.select_capture = Some("Ctrl+Shift+A".parse().unwrap())
            }),
            ("hotkeys.restore_last", |s| {
                s.hotkeys.restore_last = Some("Ctrl+Alt+Z".parse().unwrap())
            }),
            ("hotkeys.active_window_capture", |s| {
                s.hotkeys.active_window_capture = Some("Ctrl+Shift+W".parse().unwrap())
            }),
            ("shortcuts.copy", |s| {
                s.shortcuts.insert("copy".into(), None);
            }),
            ("mouse.close", |s| {
                s.mouse.insert("close".into(), None);
            }),
            ("scrap.opacity", |s| s.scrap.opacity = 40),
            ("scrap.border", |s| s.scrap.border.visible = false),
            ("scrap.border_width", |s| s.scrap.border.width = 4),
            ("scrap.border_color", |s| {
                s.scrap.border.color = Rgb(1, 2, 3)
            }),
            ("scrap.shadow", |s| s.scrap.shadow = false),
            ("annotation.color", |s| s.annotation.color = Rgb(1, 2, 3)),
            ("annotation.width", |s| s.annotation.width = 9),
            ("annotation.font_size", |s| s.annotation.font_size = 30),
            ("annotation.font", |s| s.annotation.font = "Meiryo".into()),
            ("selection.mode", |s| s.selection.mode = Mode::Precise),
            ("selection.crosshair", |s| s.selection.crosshair = true),
            ("log_level", |s| s.log_level = LogLevel::Debug),
            ("regions", |s| {
                s.regions = vec![Region {
                    name: "r".into(),
                    hotkey: None,
                    monitor: "m".into(),
                    rect: crate::geometry::Rect {
                        left: 0,
                        top: 0,
                        right: 1,
                        bottom: 1,
                    },
                    save_dir: None,
                    file_name_template: "t".into(),
                    save: true,
                    scrap: false,
                }]
            }),
        ]
    }

    #[test]
    fn every_field_is_merged_on_its_own() {
        let base = Settings::default();
        let changes = one_field_changes();
        for (gui_key, gui_change) in &changes {
            for (file_key, file_change) in &changes {
                let mut ours = base.clone();
                gui_change(&mut ours);
                let mut theirs = base.clone();
                file_change(&mut theirs);
                assert_ne!(ours, base, "{gui_key} changes nothing");

                let merged = merge(&base, &ours, &theirs);

                // 別の項目なら両方が残り、同じ項目を同じ値にしたのなら衝突ではない
                let mut expected = theirs.clone();
                gui_change(&mut expected);
                assert_eq!(merged.settings, expected, "GUI {gui_key}, file {file_key}");
                assert!(
                    merged.conflicts.is_empty(),
                    "GUI {gui_key}, file {file_key}"
                );
            }
        }
    }
}
