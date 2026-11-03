//! 設定ファイルへの書き戻し（`write`）。
//!
//! - 項目ごとに、今の文書から読める値（`load` と同じ解釈。書いていない・不正なら既定値）と書く値を比べ、
//!   違う項目だけを書き換える。手で書いたコメント・空白・並び順はそのまま残る。
//! - 書き換える項目は、値の前後の飾り（行末のコメントなど）を引き継ぐ。ない項目は表の末尾に足し、ない表は
//!   文書の末尾に足す。
//! - 項目の種類ごとの書き方は `put`（値 1 つ）と `section`（表 1 つ）に寄せる。
//! - 定型領域（`[[regions]]`）は、ファイルにない名前のものを末尾に足すだけ（書き換え・削除・並べ替えはしない）。

use std::collections::BTreeMap;
use std::fmt::Display;

use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, TableLike, Value};

use super::{
    ANNOTATION_FONT_SIZE_RANGE, ANNOTATION_WIDTH_RANGE, AnnotationDefaults, BORDER_WIDTH_RANGE,
    DEFAULT_REGION_FILE_NAME_TEMPLATE, DpiScaling, General, HotkeyAction, Hotkeys, LogLevel,
    OPACITY_RANGE, Reader, Region, ScrapAppearance, SelectionScreen, Settings, TRASH_LIMIT_RANGE,
    VERSION, dpi_scaling, in_range, language, log_level, non_empty_str, optional_chord,
    optional_gesture, read_regions, region_number, rgb, save_dir, selection_mode,
};
use crate::i18n::Language;
use crate::selection::Mode;

/// `settings` を `doc` に書く。変わった項目だけを書き換える。
pub fn write(doc: &mut DocumentMut, settings: &Settings) {
    let root = doc.as_table_mut();
    put(root, "version", Item::as_integer, None, &VERSION, |&v| {
        Some(v.into())
    });
    put(
        root,
        "log_level",
        log_level,
        Some(LogLevel::default()),
        &settings.log_level,
        |&v| Some(log_level_name(v).into()),
    );
    section(root, "general", |t| write_general(t, &settings.general));
    section(root, "hotkeys", |t| write_hotkeys(t, &settings.hotkeys));
    section(root, "shortcuts", |t| {
        write_overrides(t, &settings.shortcuts, optional_chord)
    });
    section(root, "mouse", |t| {
        write_overrides(t, &settings.mouse, optional_gesture)
    });
    section(root, "scrap", |t| write_scrap(t, &settings.scrap));
    section(root, "annotation", |t| {
        write_annotation(t, &settings.annotation)
    });
    section(root, "selection", |t| {
        write_selection(t, &settings.selection)
    });
    write_regions(root, &settings.regions);
}

/// 設定ファイルの中身 `src` に `settings` を書いた中身（`write` を文字列で行う）。ファイルがなければ `src` は空。
/// `src` が TOML として読めなければ None（上書きすると手で書いた中身が消えるので、書かない）。
pub fn rewrite(src: &str, settings: &Settings) -> Option<String> {
    let mut doc: DocumentMut = src.parse().ok()?;
    write(&mut doc, settings);
    Some(doc.to_string())
}

/// アプリが自分で書いた設定ファイルの中身（のハッシュ）。設定ファイルの監視が、自分の書いた直後の中身を
/// 拾ったときに、読み込み直しと通知を飛ばすために使う。
///
/// 自分のものでない中身を 1 度見たら忘れる。手で書き換えたあとに、自分の書いた中身へ手で戻したときは
/// 読み込み直す必要があるため。
#[derive(Debug, Default)]
pub struct OwnWrites {
    last: Option<u64>,
}

impl OwnWrites {
    /// 設定ファイルに `text` を書いた。
    pub fn wrote(&mut self, text: &str) {
        self.last = Some(hash(text));
    }

    /// 監視が拾った中身 `text` が、自分の書いた直後の中身か。違えば覚えていた中身を忘れる。
    pub fn is_own(&mut self, text: &str) -> bool {
        let own = self.last == Some(hash(text));
        if !own {
            self.last = None;
        }
        own
    }

    /// 監視が中身を読めなかった（ファイルがない、読めない）。覚えていた中身を忘れる。
    pub fn forget(&mut self) {
        self.last = None;
    }
}

/// 同じプロセスの中で比べるだけなので、`DefaultHasher` の値でよい。
fn hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

fn write_general(t: &mut dyn TableLike, g: &General) {
    let d = General::default();
    put(
        t,
        "language",
        language,
        Some(d.language),
        &g.language,
        |v| Some(v.map_or("auto", Language::code).into()),
    );
    put(
        t,
        "save_dir",
        |i| save_dir(i).map(Some),
        Some(None),
        &g.save_dir,
        |dir| {
            dir.as_ref()
                .map(|p| p.to_string_lossy().into_owned().into())
        },
    );
    put(
        t,
        "file_name_template",
        non_empty_str,
        Some(d.file_name_template),
        &g.file_name_template,
        |s| Some(s.as_str().into()),
    );
    put(
        t,
        "notifications",
        Item::as_bool,
        Some(d.notifications),
        &g.notifications,
        |&v| Some(v.into()),
    );
    put(
        t,
        "trash_limit",
        |i| in_range(i, TRASH_LIMIT_RANGE),
        Some(d.trash_limit),
        &g.trash_limit,
        |&v| Some(i64::from(v).into()),
    );
    put(
        t,
        "dpi_scaling",
        dpi_scaling,
        Some(d.dpi_scaling),
        &g.dpi_scaling,
        |&v| Some(dpi_scaling_name(v).into()),
    );
    put(
        t,
        "capture_cursor",
        Item::as_bool,
        Some(d.capture_cursor),
        &g.capture_cursor,
        |&v| Some(v.into()),
    );
}

/// 空のホットキーは項目を消さずに `""` と書く（行と行末のコメントを残すため）。
fn write_hotkeys(t: &mut dyn TableLike, h: &Hotkeys) {
    for action in HotkeyAction::ALL {
        put(
            t,
            action.key(),
            optional_chord,
            Some(None),
            &h.get(action),
            |c| Some(optional_str(c).into()),
        );
    }
}

/// 操作の id ごとの上書き（`[shortcuts]`・`[mouse]`）。`overrides` にない id は消す。None は `""` と書く。
fn write_overrides<T: PartialEq + Display>(
    t: &mut dyn TableLike,
    overrides: &BTreeMap<String, Option<T>>,
    parse: fn(&Item) -> Option<Option<T>>,
) {
    let stale: Vec<String> = t
        .iter()
        .map(|(id, _)| id.to_owned())
        .filter(|id| !overrides.contains_key(id))
        .collect();
    for id in stale {
        t.remove(&id);
    }
    for (id, value) in overrides {
        put(t, id, parse, None, value, |v| Some(optional_str(v).into()));
    }
}

fn write_scrap(t: &mut dyn TableLike, s: &ScrapAppearance) {
    let d = ScrapAppearance::default();
    put(
        t,
        "opacity",
        |i| in_range(i, OPACITY_RANGE),
        Some(d.opacity),
        &s.opacity,
        |&v| Some(i64::from(v).into()),
    );
    put(
        t,
        "border",
        Item::as_bool,
        Some(d.border.visible),
        &s.border.visible,
        |&v| Some(v.into()),
    );
    put(
        t,
        "border_width",
        |i| in_range(i, BORDER_WIDTH_RANGE),
        Some(d.border.width),
        &s.border.width,
        |&v| Some(i64::from(v).into()),
    );
    put(
        t,
        "border_color",
        rgb,
        Some(d.border.color),
        &s.border.color,
        |c| Some(c.to_string().into()),
    );
    put(
        t,
        "shadow",
        Item::as_bool,
        Some(d.shadow),
        &s.shadow,
        |&v| Some(v.into()),
    );
}

fn write_annotation(t: &mut dyn TableLike, a: &AnnotationDefaults) {
    let d = AnnotationDefaults::default();
    put(t, "color", rgb, Some(d.color), &a.color, |c| {
        Some(c.to_string().into())
    });
    put(
        t,
        "width",
        |i| in_range(i, ANNOTATION_WIDTH_RANGE),
        Some(d.width),
        &a.width,
        |&v| Some(i64::from(v).into()),
    );
    put(
        t,
        "font_size",
        |i| in_range(i, ANNOTATION_FONT_SIZE_RANGE),
        Some(d.font_size),
        &a.font_size,
        |&v| Some(i64::from(v).into()),
    );
    put(t, "font", non_empty_str, Some(d.font), &a.font, |s| {
        Some(s.as_str().into())
    });
}

fn write_selection(t: &mut dyn TableLike, s: &SelectionScreen) {
    let d = SelectionScreen::default();
    put(t, "mode", selection_mode, Some(d.mode), &s.mode, |&m| {
        Some(selection_mode_name(m).into())
    });
    put(
        t,
        "crosshair",
        Item::as_bool,
        Some(d.crosshair),
        &s.crosshair,
        |&v| Some(v.into()),
    );
}

/// 定型領域（`[[regions]]`）。ファイルにない名前の定型領域を、最後の要素の後ろに足す（アプリから書くのは
/// 定型領域の登録だけ。#71）。ファイルにある要素（読み込みで捨てられるものも）は、手で書いたまま残す。
fn write_regions(root: &mut Table, regions: &[Region]) {
    let tables = match root
        .get_mut("regions")
        .and_then(Item::as_array_of_tables_mut)
    {
        Some(tables) => tables,
        None if regions.is_empty() => return,
        None => {
            root.insert("regions", Item::ArrayOfTables(ArrayOfTables::new()));
            root.get_mut("regions")
                .and_then(Item::as_array_of_tables_mut)
                .expect("just inserted")
        }
    };
    let kept: Vec<String> = read_regions(tables, &mut Reader { issues: Vec::new() })
        .into_iter()
        .map(|region| region.name)
        .collect();
    for region in regions.iter().filter(|r| !kept.contains(&r.name)) {
        let mut table = Table::new();
        write_region(&mut table, region);
        tables.push(table);
    }
}

/// 定型領域 1 つ。名前・モニタ・範囲は既定値がないので、読めなければ常に書く。
fn write_region(t: &mut dyn TableLike, r: &Region) {
    put(t, "name", non_empty_str, None, &r.name, |s| {
        Some(s.as_str().into())
    });
    put(t, "hotkey", optional_chord, Some(None), &r.hotkey, |c| {
        Some(optional_str(c).into())
    });
    put(t, "monitor", non_empty_str, None, &r.monitor, |s| {
        Some(s.as_str().into())
    });
    let size = |n: u32| i32::try_from(n).unwrap_or(i32::MAX);
    for (key, min, n) in [
        ("x", 0, r.rect.left),
        ("y", 0, r.rect.top),
        ("width", 1, size(r.rect.width())),
        ("height", 1, size(r.rect.height())),
    ] {
        put(
            t,
            key,
            |i| region_number(i, min),
            None,
            &n,
            |&n| Some(i64::from(n).into()),
        );
    }
    put(
        t,
        "save_dir",
        |i| save_dir(i).map(Some),
        Some(None),
        &r.save_dir,
        |dir| {
            dir.as_ref()
                .map(|p| p.to_string_lossy().into_owned().into())
        },
    );
    put(
        t,
        "file_name_template",
        non_empty_str,
        Some(DEFAULT_REGION_FILE_NAME_TEMPLATE.to_owned()),
        &r.file_name_template,
        |s| Some(s.as_str().into()),
    );
    put(t, "save", Item::as_bool, Some(true), &r.save, |&v| {
        Some(v.into())
    });
    put(t, "scrap", Item::as_bool, Some(false), &r.scrap, |&v| {
        Some(v.into())
    });
}

/// 表 `name` の中を `f` で書く。表がなければ（表でない値なら置き換えて）、何か書いたときだけ文書の末尾に足す。
fn section(root: &mut Table, name: &str, f: impl FnOnce(&mut dyn TableLike)) {
    if let Some(table) = root.get_mut(name).and_then(Item::as_table_like_mut) {
        f(table);
        return;
    }
    let mut table = Table::new();
    f(&mut table);
    if !table.is_empty() {
        root.insert(name, Item::Table(table));
    }
}

/// 表 `table` の項目 `key` に `new` を書く。
///
/// - 今の値は `parse` で読む（`load` と同じ解釈）。書いていないか不正なら `default`。`default` が None の
///   項目（`[shortcuts]` の上書きのように、書いていないことに意味があるもの）は、書いていなければ常に書く。
/// - 今の値が `new` と同じなら何もしない（不正な書き方もそのまま残す）。
/// - `value` が None を返したら、項目を消す（`save_dir` を既定に戻すときなど）。
fn put<T: PartialEq>(
    table: &mut dyn TableLike,
    key: &str,
    parse: impl FnOnce(&Item) -> Option<T>,
    default: Option<T>,
    new: &T,
    value: impl FnOnce(&T) -> Option<Value>,
) {
    let current = match table.get(key) {
        Some(item) => parse(item).or(default),
        None => default,
    };
    if current.as_ref() == Some(new) {
        return;
    }
    match value(new) {
        None => {
            table.remove(key);
        }
        Some(mut v) => match table.get_mut(key).and_then(Item::as_value_mut) {
            Some(old) => {
                *v.decor_mut() = old.decor().clone();
                *old = v;
            }
            None => {
                table.insert(key, Item::Value(v));
            }
        },
    }
}

/// 空は `""`。
fn optional_str<T: Display>(value: &Option<T>) -> String {
    value.as_ref().map(T::to_string).unwrap_or_default()
}

fn dpi_scaling_name(v: DpiScaling) -> &'static str {
    match v {
        DpiScaling::KeepPixels => "keep_pixels",
        DpiScaling::KeepApparentSize => "keep_apparent_size",
    }
}

fn selection_mode_name(v: Mode) -> &'static str {
    match v {
        Mode::Normal => "normal",
        Mode::Precise => "precise",
    }
}

fn log_level_name(v: LogLevel) -> &'static str {
    match v {
        LogLevel::Off => "off",
        LogLevel::Error => "error",
        LogLevel::Warn => "warn",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
        LogLevel::Trace => "trace",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::geometry::Rect;
    use crate::keys::Chord;
    use crate::scrap::ops::MouseGesture;
    use crate::settings::{Rgb, load};

    /// `src` を読み、`change` で変えた設定を書いた文字列。
    fn written(src: &str, change: impl FnOnce(&mut Settings)) -> String {
        let (mut settings, _) = load(src);
        change(&mut settings);
        let mut doc: DocumentMut = src.parse().unwrap();
        write(&mut doc, &settings);
        doc.to_string()
    }

    #[test]
    fn only_the_changed_item_is_rewritten_and_comments_and_order_are_kept() {
        let src = r#"# 手で書いた設定
version = 1

[scrap]
shadow = false   # 影はいらない
# 半透明にする
opacity = 80     # 80%

[general]
notifications = false
"#;
        let out = written(src, |s| s.scrap.opacity = 50);
        assert_eq!(out, src.replace("opacity = 80", "opacity = 50"));
        assert_eq!(load(&out).0.scrap.opacity, 50);
    }

    fn chord(s: &str) -> Option<Chord> {
        Some(s.parse().unwrap())
    }

    /// 既定値からすべての項目を変えた設定。
    fn every_item_changed() -> Settings {
        let mut s = Settings {
            log_level: LogLevel::Debug,
            ..Settings::default()
        };
        s.general.language = Some(crate::i18n::Language::English);
        s.general.save_dir = Some(PathBuf::from(r"D:\shots"));
        s.general.file_name_template = "cap_{seq:3}".to_owned();
        s.general.notifications = false;
        s.general.trash_limit = 0;
        s.general.dpi_scaling = DpiScaling::KeepApparentSize;
        s.general.capture_cursor = true;
        s.hotkeys.select_capture = chord("Ctrl+Shift+A");
        s.hotkeys.restore_last = chord("Win+Alt+Z");
        s.shortcuts.insert("copy".to_owned(), chord("Ctrl+Shift+C"));
        s.shortcuts.insert("close".to_owned(), None);
        s.mouse
            .insert("close".to_owned(), Some(MouseGesture::DoubleClick));
        s.mouse.insert("zoom_in".to_owned(), None);
        s.scrap.opacity = 60;
        s.scrap.border.visible = false;
        s.scrap.border.width = 4;
        s.scrap.border.color = Rgb(0x12, 0xAB, 0xEF);
        s.scrap.shadow = false;
        s.annotation.color = Rgb(0x1E, 0x88, 0xE5);
        s.annotation.width = 8;
        s.annotation.font_size = 24;
        s.annotation.font = "Meiryo".to_owned();
        s.selection.mode = Mode::Precise;
        s.selection.crosshair = true;
        s
    }

    #[test]
    fn every_item_written_into_a_file_without_it_reads_back_the_same() {
        let wanted = every_item_changed();
        // 項目のない文書のコメントは文書の末尾のものとして扱われるので、足した項目の後ろに残る
        let out = written("# 空の設定\n", |s| *s = wanted.clone());
        assert!(out.contains("# 空の設定\n"), "{out}");
        assert_eq!(load(&out), (wanted, vec![]));
    }

    #[test]
    fn the_sample_file_keeps_every_line_when_an_item_is_added() {
        let sample = include_str!("../../../../docs/settings.sample.toml");
        let out = written(sample, |s| {
            s.hotkeys.select_capture = chord("Ctrl+Shift+A");
            s.general.save_dir = Some(PathBuf::from(r"D:\shots"));
        });
        let (read, issues) = load(&out);
        assert_eq!(issues, vec![]);
        assert_eq!(read.hotkeys.select_capture, chord("Ctrl+Shift+A"));
        assert_eq!(read.general.save_dir, Some(PathBuf::from(r"D:\shots")));
        // 足した 2 行のほかは、サンプルの行がそのままの順で残る
        let added = [
            "select_capture = \"Ctrl+Shift+A\"",
            r"save_dir = 'D:\shots'",
        ];
        let kept: Vec<&str> = out.lines().filter(|l| !added.contains(l)).collect();
        assert_eq!(kept, sample.lines().collect::<Vec<_>>());
        assert_eq!(out.lines().count(), sample.lines().count() + 2);
    }

    #[test]
    fn writing_the_settings_that_were_read_changes_nothing() {
        let src = r##"version = 1
log_level = "DEBUG"   # 大文字でも読める
[general]
save_dir = 'D:/shots'
trash_limit = 3
[hotkeys]
select_capture = "ctrl+shift+a"
restore_last = ""
[shortcuts]
copy = "Ctrl+Shift+C"
close = ""
[mouse]
close = "DoubleClick"
[scrap]
border_color = "#1a2b3c"
opacity = 999     # 不正な値は既定値として読まれ、書いても残る
"##;
        assert_eq!(written(src, |_| {}), src);
    }

    #[test]
    fn items_set_back_to_the_defaults_keep_their_lines() {
        let src = r##"version = 1
[general]
notifications = false # 通知は要らない
[hotkeys]
select_capture = "Ctrl+Shift+A"
[scrap]
opacity = 50
"##;
        let out = written(src, |s| *s = Settings::default());
        assert_eq!(
            out,
            r##"version = 1
[general]
notifications = true # 通知は要らない
[hotkeys]
select_capture = ""
[scrap]
opacity = 100
"##
        );
    }

    #[test]
    fn the_language_set_back_to_follow_the_os_is_written_as_auto() {
        let src = "version = 1\n[general]\nlanguage = \"en\" # 英語にする\n";
        let out = written(src, |s| s.general.language = None);
        assert_eq!(
            out,
            "version = 1\n[general]\nlanguage = \"auto\" # 英語にする\n"
        );
        let out = written(src, |s| {
            s.general.language = Some(crate::i18n::Language::Japanese)
        });
        assert_eq!(
            out,
            "version = 1\n[general]\nlanguage = \"ja\" # 英語にする\n"
        );
    }

    #[test]
    fn the_save_dir_set_back_to_the_default_is_removed() {
        let src = "version = 1\n[general]\nsave_dir = 'D:\\shots'\nnotifications = false\n";
        let out = written(src, |s| s.general.save_dir = None);
        assert_eq!(out, "version = 1\n[general]\nnotifications = false\n");
    }

    #[test]
    fn overrides_no_longer_in_the_settings_are_removed() {
        let src = r#"version = 1
[shortcuts]
copy = "Ctrl+Shift+C" # コピー
close = ""
[mouse]
close = "DoubleClick"
"#;
        let out = written(src, |s| {
            s.shortcuts.remove("close");
            s.shortcuts.insert("paste".to_owned(), chord("Ctrl+V"));
            s.mouse.clear();
        });
        assert_eq!(
            out,
            r#"version = 1
[shortcuts]
copy = "Ctrl+Shift+C" # コピー
paste = "Ctrl+V"
[mouse]
"#
        );
    }

    #[test]
    fn content_the_app_wrote_is_recognised_until_other_content_is_seen() {
        let mut own = OwnWrites::default();
        assert!(!own.is_own("version = 1\n"));
        own.wrote("version = 1\n[scrap]\nshadow = false\n");
        // 1 回の書き込みで監視が何度か拾っても、同じ中身なら自分のもの
        assert!(own.is_own("version = 1\n[scrap]\nshadow = false\n"));
        assert!(own.is_own("version = 1\n[scrap]\nshadow = false\n"));
        // 手で書き換えた中身は読み込む。そのあと手で元に戻した中身も、自分のものとは見なさない
        assert!(!own.is_own("version = 1\n[scrap]\nshadow = true\n"));
        assert!(!own.is_own("version = 1\n[scrap]\nshadow = false\n"));
    }

    #[test]
    fn rewriting_text_writes_into_it_and_refuses_text_that_is_not_toml() {
        let s = Settings {
            scrap: ScrapAppearance {
                shadow: false,
                ..ScrapAppearance::default()
            },
            ..Settings::default()
        };
        assert_eq!(
            rewrite("version = 1 # 版\n", &s).as_deref(),
            Some("version = 1 # 版\n\n[scrap]\nshadow = false\n")
        );
        // 読めないファイルを上書きすると手で書いた中身が消えるので、書かない
        assert_eq!(rewrite("version = = 1\n", &s), None);
    }

    fn region(name: &str) -> Region {
        Region {
            name: name.to_owned(),
            hotkey: None,
            monitor: r"\\?\DISPLAY#AUS2701#5&1".to_owned(),
            rect: Rect::new(10, 20, 650, 500),
            save_dir: None,
            file_name_template: DEFAULT_REGION_FILE_NAME_TEMPLATE.to_owned(),
            save: true,
            scrap: false,
        }
    }

    #[test]
    fn a_region_added_to_a_file_reads_back_the_same() {
        let src = "version = 1 # 版\n\n[scrap]\nshadow = false\n";
        let added = Region {
            hotkey: chord("Ctrl+Alt+1"),
            save_dir: Some(PathBuf::from(r"D:\shots")),
            file_name_template: "{name}_{seq:3}".to_owned(),
            save: false,
            scrap: true,
            ..region("領域 1")
        };
        let out = written(src, |s| s.regions.push(added.clone()));
        let (read, issues) = load(&out);
        assert_eq!(issues, vec![]);
        assert_eq!(read.regions, vec![added]);
        assert!(out.starts_with(src), "{out}");
    }

    /// 手で書いた定型領域 2 つと、その後ろの表。
    const TWO_REGIONS: &str = r#"version = 1

# 定型領域
[[regions]]
name = "a"            # 左上
monitor = 'M1'
x = 0
y = 0
width = 10
height = 10

[[regions]]
# 右下
name = "b"
monitor = 'M1'
x = 100
y = 100
width = 20 # 幅
height = 20

[scrap]
shadow = false
"#;

    #[test]
    fn a_region_added_after_others_goes_after_the_last_region() {
        let added = Region {
            monitor: "M2".to_owned(),
            rect: Rect::new(1, 2, 4, 6),
            ..region("c")
        };
        let out = written(TWO_REGIONS, |s| s.regions.push(added.clone()));
        assert_eq!(
            out,
            TWO_REGIONS.replace(
                "height = 20\n",
                "height = 20\n\n[[regions]]\nname = \"c\"\nmonitor = \"M2\"\nx = 1\ny = 2\nwidth = 3\nheight = 4\n"
            )
        );
        assert_eq!(load(&out).0.regions[2], added);
    }

    #[test]
    fn regions_dropped_when_read_are_left_in_place() {
        let src = r#"version = 1
[[regions]]
name = "a"
monitor = 'M1'
x = 0
y = 0
width = 10
height = 10
[[regions]]
name = "書きかけ"
monitor = 'M1'
[[regions]]
name = "b"
monitor = 'M1'
x = 0
y = 0
width = 10
height = 10
"#;
        assert_eq!(written(src, |_| {}), src);
        let out = written(src, |s| s.regions.push(region("c")));
        assert!(out.starts_with(src), "{out}");
        let names: Vec<String> = load(&out).0.regions.into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn regions_that_are_not_an_array_of_tables_are_replaced_only_when_regions_are_written() {
        assert_eq!(
            written("version = 1\nregions = 3\n", |_| {}),
            "version = 1\nregions = 3\n"
        );
        let out = written("version = 1\nregions = 3\n", |s| {
            s.regions.push(region("a"))
        });
        let wanted = Settings {
            regions: vec![region("a")],
            ..Settings::default()
        };
        assert_eq!(load(&out), (wanted, vec![]));
    }

    #[test]
    fn each_annotation_item_rewrites_only_itself_and_keeps_comments() {
        let src = r##"version = 1
# 書き込み
[annotation]
color = "#1E88E5"   # 青
width = 8 # 太め
font_size = 24
font = "Meiryo"     # 書体
"##;
        assert_eq!(
            written(src, |s| s.annotation.color = Rgb(0x43, 0xA0, 0x47)),
            src.replace("#1E88E5", "#43A047")
        );
        assert_eq!(
            written(src, |s| s.annotation.width = 2),
            src.replace("width = 8", "width = 2")
        );
        assert_eq!(
            written(src, |s| s.annotation.font_size = 12),
            src.replace("font_size = 24", "font_size = 12")
        );
        assert_eq!(
            written(src, |s| s.annotation.font = "Yu Gothic UI".to_owned()),
            src.replace("\"Meiryo\"", "\"Yu Gothic UI\"")
        );
    }

    #[test]
    fn annotation_defaults_written_into_a_file_without_them_read_back_the_same() {
        let out = written("version = 1\n", |s| {
            s.annotation.color = Rgb(0, 0, 0);
            s.annotation.font = "Meiryo".to_owned();
        });
        assert_eq!(
            out,
            "version = 1\n\n[annotation]\ncolor = \"#000000\"\nfont = \"Meiryo\"\n"
        );
        let read = load(&out).0.annotation;
        assert_eq!(read.color, Rgb(0, 0, 0));
        assert_eq!(read.font, "Meiryo");
    }

    #[test]
    fn a_missing_version_is_written() {
        let out = written("[scrap]\nshadow = false\n", |_| {});
        assert_eq!(load(&out).1, vec![]);
    }
}
