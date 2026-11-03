//! 設定 GUI の適用での 3 者の突き合わせ（`merge`）。
//!
//! - base（開いた時点か最後に適用した時点の設定）、ours（GUI の写し）、theirs（適用の時点でファイルから読んだ
//!   設定）を項目（`Field`）ごとに比べる。GUI で変えた項目（base と ours が違う）だけを theirs に当て、外で
//!   変えた項目は theirs のまま残す。
//! - 外と GUI の両方で同じ項目を違う値に変えていたら衝突（`Merged::conflicts`）。`Merged::settings` は衝突を
//!   GUI の値にしたもの（「GUI の値で上書き」）で、衝突があれば書く前に確かめる。
//! - 項目は値の 1 つずつ（`[shortcuts]` と `[mouse]` は操作の id ごと）。`Settings` に項目を足したら、`merge`
//!   の分解（`..` なし）がコンパイルを止めるので、ここに 1 行足す。
//! - 定型領域は 1 つずつ、項目ごとに突き合わせる（`merge_regions`）。定型領域は base と theirs ではファイルでの
//!   名前（`Region::loaded_name`）、ours では GUI で開いたときの名前（`loaded_name`。None は GUI で足したもの）で
//!   見分ける。片方で消し、もう片方で変えていたら衝突（`Field::Region`）。並びは GUI で並べ替えていれば GUI の、
//!   そうでなければファイルの並びにし、片方にしかない定型領域は、その側で前にあった定型領域の後ろに入れる。

use std::collections::BTreeMap;

use super::{
    AnnotationDefaults, Border, General, Hotkeys, Region, ScrapAppearance, SelectionScreen,
    Settings,
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
    /// 定型領域 1 つ（base での名前）。片方で消し、もう片方で変えた。または両方で同じ名前にした。
    Region(String),
    /// 定型領域 1 つ（base での名前）の項目（`name`、`hotkey`、`monitor`、`rect` など）。
    RegionKey(String, &'static str),
    /// 定型領域の並び。
    RegionOrder,
}

impl std::fmt::Display for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Field::Key(key) => f.write_str(key),
            Field::Shortcut(id) => write!(f, "shortcuts.{id}"),
            Field::Mouse(id) => write!(f, "mouse.{id}"),
            Field::Region(name) => write!(f, "regions.{name}"),
            Field::RegionKey(name, key) => write!(f, "regions.{name}.{key}"),
            Field::RegionOrder => f.write_str("regions"),
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
        language,
        save_dir,
        file_name_template,
        autostart,
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
    value!("general.language", language, general.language);
    value!("general.save_dir", save_dir, general.save_dir);
    value!(
        "general.file_name_template",
        file_name_template,
        general.file_name_template
    );
    value!("general.autostart", autostart, general.autostart);
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
    out.regions = merge_regions(&base.regions, regions, &theirs.regions, &mut conflicts);

    Merged {
        settings: out,
        conflicts,
    }
}

/// 定型領域の並びを突き合わせる（モジュールの説明）。
fn merge_regions(
    base: &[Region],
    ours: &[Region],
    theirs: &[Region],
    conflicts: &mut Vec<Field>,
) -> Vec<Region> {
    let file_name = |r: &Region| r.loaded_name.clone().unwrap_or_else(|| r.name.clone());
    let in_base = |id: &str| base.iter().any(|b| file_name(b) == id);
    // 結果の定型領域。キーは、base にあるものは base での名前、ほかは側ごとの添字
    let mut merged: BTreeMap<Key, Region> = BTreeMap::new();
    for b in base {
        let id = file_name(b);
        let o = ours.iter().find(|o| o.loaded_name.as_deref() == Some(&id));
        let t = theirs.iter().find(|t| file_name(t) == id);
        match (o, t) {
            (Some(o), Some(t)) => {
                let mut region = t.clone();
                merge_region(b, o, &mut region, &id, conflicts);
                merged.insert(Key::Base(id), region);
            }
            // GUI で消した。外で変えていれば衝突（消す）
            (None, Some(t)) if t != b => conflicts.push(Field::Region(id)),
            // 外で消した。GUI で変えていれば衝突（GUI の値で足し直す）
            (Some(o), None) if o != b => {
                conflicts.push(Field::Region(id.clone()));
                let region = Region {
                    loaded_name: None,
                    ..o.clone()
                };
                merged.insert(Key::Base(id), region);
            }
            _ => {}
        }
    }
    let ours_keys: Vec<Key> = ours
        .iter()
        .enumerate()
        .map(|(i, o)| match &o.loaded_name {
            Some(id) if in_base(id) => Key::Base(id.clone()),
            _ => Key::Ours(i),
        })
        .collect();
    let theirs_keys: Vec<Key> = theirs
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let id = file_name(t);
            if in_base(&id) {
                Key::Base(id)
            } else {
                Key::Theirs(i)
            }
        })
        .collect();
    for (key, o) in ours_keys.iter().zip(ours) {
        if let Key::Ours(_) = key {
            merged.insert(key.clone(), o.clone());
        }
    }
    for (key, t) in theirs_keys.iter().zip(theirs) {
        if let Key::Theirs(_) = key {
            merged.insert(key.clone(), t.clone());
        }
    }

    // 並び
    let base_keys: Vec<Key> = base.iter().map(|b| Key::Base(file_name(b))).collect();
    let gui_reordered = reordered(&base_keys, &ours_keys);
    if gui_reordered && reordered(&base_keys, &theirs_keys) && reordered(&ours_keys, &theirs_keys) {
        conflicts.push(Field::RegionOrder);
    }
    let order = if gui_reordered {
        interleave(&ours_keys, &theirs_keys)
    } else {
        interleave(&theirs_keys, &ours_keys)
    };

    // 両方で同じ名前にしたら、GUI の側（ours にあるもの）を残す。中身まで同じなら衝突ではない
    let mut out: Vec<(Key, Region)> = Vec::new();
    for key in order {
        let Some(region) = merged.remove(&key) else {
            continue;
        };
        let from_gui = ours_keys.contains(&key);
        match out.iter().position(|(_, r)| r.name == region.name) {
            Some(j) => {
                if out[j].1 != region {
                    conflicts.push(Field::Region(region.name.clone()));
                }
                if from_gui && !ours_keys.contains(&out[j].0) {
                    out[j] = (key, region);
                }
            }
            None => out.push((key, region)),
        }
    }
    out.into_iter().map(|(_, region)| region).collect()
}

/// 突き合わせの中で定型領域を見分けるキー。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    /// base にある定型領域（base での名前）。
    Base(String),
    /// GUI で足した定型領域（ours での添字）。
    Ours(usize),
    /// 外で足した定型領域（theirs での添字）。
    Theirs(usize),
}

/// `a` と `b` の両方にあるキーの順が違うか。
fn reordered(a: &[Key], b: &[Key]) -> bool {
    let common = |x: &[Key], y: &[Key]| -> Vec<Key> {
        x.iter().filter(|k| y.contains(k)).cloned().collect()
    };
    common(a, b) != common(b, a)
}

/// `primary` の並びに、`secondary` にしかないキーを、`secondary` でその前にあったキーの後ろ（なければ先頭）に入れる。
fn interleave(primary: &[Key], secondary: &[Key]) -> Vec<Key> {
    let mut out: Vec<Key> = primary.to_vec();
    for (i, key) in secondary.iter().enumerate() {
        if out.contains(key) {
            continue;
        }
        let at = secondary[..i]
            .iter()
            .rev()
            .find_map(|before| out.iter().position(|k| k == before))
            .map_or(0, |j| j + 1);
        out.insert(at, key.clone());
    }
    out
}

/// 両方にある定型領域 1 つを項目ごとに突き合わせる。`out` は theirs の定型領域。
fn merge_region(
    base: &Region,
    ours: &Region,
    out: &mut Region,
    id: &str,
    conflicts: &mut Vec<Field>,
) {
    // 項目を足したらここで止まる（`..` を使わない）
    let Region {
        name,
        hotkey,
        monitor,
        rect,
        save_dir,
        file_name_template,
        save,
        scrap,
        loaded_name: _,
    } = ours;
    let mut m = |key: &'static str, (changed, both): (bool, bool)| {
        if changed && both {
            conflicts.push(Field::RegionKey(id.to_owned(), key));
        }
    };
    m("name", take(&base.name, name, &mut out.name));
    m("hotkey", take(&base.hotkey, hotkey, &mut out.hotkey));
    m("monitor", take(&base.monitor, monitor, &mut out.monitor));
    m("rect", take(&base.rect, rect, &mut out.rect));
    m(
        "save_dir",
        take(&base.save_dir, save_dir, &mut out.save_dir),
    );
    m(
        "file_name_template",
        take(
            &base.file_name_template,
            file_name_template,
            &mut out.file_name_template,
        ),
    );
    m("save", take(&base.save, save, &mut out.save));
    m("scrap", take(&base.scrap, scrap, &mut out.scrap));
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

    /// 定型領域 `names` を書いたファイルを読んだ設定（`loaded_name` が付く）。
    fn regions(names: &[&str]) -> Settings {
        let mut src = String::from("version = 1\n");
        for name in names {
            src.push_str(&format!(
                "[[regions]]\nname = \"{name}\"\nmonitor = \"m\"\nx = 0\ny = 0\nwidth = 10\nheight = 10\n"
            ));
        }
        let (settings, issues) = crate::settings::load(&src);
        assert_eq!(issues, vec![]);
        settings
    }

    fn names(s: &Settings) -> Vec<&str> {
        s.regions.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn regions_are_merged_per_region_and_per_item() {
        let base = regions(&["a", "b"]);
        let mut ours = base.clone();
        ours.regions[0].hotkey = Some("Ctrl+1".parse().unwrap()); // GUI だけ
        ours.regions[0].name = "a2".into(); // GUI で名前を変える
        let mut theirs = base.clone();
        theirs.regions[0].save_dir = Some("D:\\a".into()); // 同じ領域の別の項目を外で
        theirs.regions[1].scrap = true; // 別の領域を外で

        let merged = merge(&base, &ours, &theirs);

        assert_eq!(merged.conflicts, vec![]);
        let r = &merged.settings.regions;
        assert_eq!(names(&merged.settings), ["a2", "b"]);
        assert_eq!(r[0].hotkey, Some("Ctrl+1".parse().unwrap()));
        assert_eq!(r[0].save_dir, Some("D:\\a".into()));
        assert!(r[1].scrap);
        // ファイルの要素に結び付けたまま（書くと名前を変えた要素のコメントが残る）
        assert_eq!(r[0].loaded_name.as_deref(), Some("a"));
    }

    #[test]
    fn the_same_region_item_changed_on_both_sides_conflicts_and_the_gui_value_wins() {
        let base = regions(&["a", "b"]);
        let mut ours = base.clone();
        ours.regions[1].rect.left = 5;
        let mut theirs = base.clone();
        theirs.regions[1].rect.left = 7;

        let merged = merge(&base, &ours, &theirs);

        assert_eq!(merged.conflicts, vec![Field::RegionKey("b".into(), "rect")]);
        assert_eq!(merged.conflicts[0].to_string(), "regions.b.rect");
        assert_eq!(merged.settings.regions[1].rect.left, 5);
    }

    #[test]
    fn regions_added_and_deleted_on_either_side_are_kept_apart() {
        let base = regions(&["a", "b", "c"]);
        let mut ours = base.clone();
        ours.regions.remove(0); // GUI で消す
        let mut added = ours.regions[0].clone();
        added.name = "g".into();
        added.loaded_name = None;
        ours.regions.insert(1, added); // GUI で b の後ろに足す
        let mut theirs = regions(&["a", "x", "b"]); // 外で c を消し、x を足す

        let merged = merge(&base, &ours, &theirs);
        assert_eq!(merged.conflicts, vec![]);
        assert_eq!(names(&merged.settings), ["x", "b", "g"]);

        // 片方で消し、もう片方で変えていたら衝突。GUI の側（消す・変えたまま残す）にする
        theirs = base.clone();
        theirs.regions[0].scrap = true;
        theirs.regions.remove(2);
        let mut ours = base.clone();
        ours.regions.remove(0);
        ours.regions[1].scrap = true;
        let merged = merge(&base, &ours, &theirs);
        assert_eq!(
            merged.conflicts,
            vec![Field::Region("a".into()), Field::Region("c".into())]
        );
        assert_eq!(names(&merged.settings), ["b", "c"]);
        assert!(merged.settings.regions[1].scrap);
    }

    #[test]
    fn the_gui_order_is_taken_when_the_gui_reordered() {
        let base = regions(&["a", "b", "c"]);
        let mut ours = base.clone();
        ours.regions.swap(0, 2); // c b a
        let theirs = regions(&["a", "b", "x", "c"]);

        let merged = merge(&base, &ours, &theirs);
        assert_eq!(merged.conflicts, vec![]);
        assert_eq!(names(&merged.settings), ["c", "b", "x", "a"]);

        // 外でも並べ替えていたら衝突
        let theirs = regions(&["b", "a", "c"]);
        let merged = merge(&base, &ours, &theirs);
        assert_eq!(merged.conflicts, vec![Field::RegionOrder]);
        assert_eq!(names(&merged.settings), ["c", "b", "a"]);
    }

    #[test]
    fn a_name_taken_on_both_sides_conflicts_and_the_gui_region_stays() {
        let base = regions(&["a"]);
        let mut ours = base.clone();
        ours.regions[0].name = "x".into();
        let mut theirs = regions(&["a", "x"]);
        theirs.regions[1].scrap = true;

        let merged = merge(&base, &ours, &theirs);
        assert_eq!(merged.conflicts, vec![Field::Region("x".into())]);
        assert!(!merged.settings.regions[0].scrap);
        assert_eq!(names(&merged.settings), ["x"]);
        assert_eq!(merged.settings.regions[0].loaded_name.as_deref(), Some("a"));
    }

    /// 項目のキーと、その項目だけを変える関数。
    type Change = (&'static str, fn(&mut Settings));

    /// 1 つの項目だけを既定値から変えるもの。`Settings` に項目を足したら、ここにも足す（`merge` の分解は
    /// コンパイルで止まるが、この一覧は止まらない）。
    fn one_field_changes() -> Vec<Change> {
        use crate::selection::Mode;
        use crate::settings::{DpiScaling, LogLevel, Region, Rgb};
        vec![
            ("general.language", |s| {
                s.general.language = Some(crate::i18n::Language::English)
            }),
            ("general.save_dir", |s| {
                s.general.save_dir = Some("D:\\shots".into())
            }),
            ("general.file_name_template", |s| {
                s.general.file_name_template = "x_{date}".into()
            }),
            ("general.autostart", |s| s.general.autostart = true),
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
                    loaded_name: None,
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
