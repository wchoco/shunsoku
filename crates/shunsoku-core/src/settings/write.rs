//! 設定ファイルへの書き戻し（`write`）。
//!
//! - 項目ごとに、今の文書から読める値（`load` と同じ解釈。書いていない・不正なら既定値）と書く値を比べ、
//!   違う項目だけを書き換える。手で書いたコメント・空白・並び順はそのまま残る。
//! - 書き換える項目は、値の前後の飾り（行末のコメントなど）を引き継ぐ。ない項目は表の末尾に足し、ない表は
//!   文書の末尾に足す。
//! - 項目の種類ごとの書き方は `put`（値 1 つ）と `section`（表 1 つ）に寄せる。配列の表（`[[…]]`）の要素も
//!   `toml_edit::Table` なので、要素ごとに同じ `put` で書ける。

use std::collections::BTreeMap;
use std::fmt::Display;

use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, TableLike, Value};

use super::{
    BORDER_WIDTH_RANGE, DEFAULT_REGION_FILE_NAME_TEMPLATE, DpiScaling, General, HotkeyAction,
    Hotkeys, LogLevel, OPACITY_RANGE, Reader, Region, Rgb, ScrapAppearance, Settings,
    TRASH_LIMIT_RANGE, VERSION, dpi_scaling, in_range, log_level, non_empty_str, optional_chord,
    optional_gesture, read_regions, region_number, rgb, save_dir,
};

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
        |c| Some(rgb_str(c).into()),
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

/// 定型領域（`[[regions]]`）。書く定型領域ごとに、ファイルの中の要素（読み込みで残るもの）を 1 つ対応させ、
/// その要素に項目を書く。手で書いたコメントは要素ごと引き継がれる。
///
/// - 対応: 同じ名前の要素。残りは、名前の合わない要素と並び順で（名前を変えた定型領域）。どれにも対応しない
///   定型領域は要素を新しく作り、どれにも対応しない要素は消す。
/// - 並び: 対応のついた要素のあった場所に、書く定型領域を順に置く。場所が足りなければ最後の要素の後ろに足す。
/// - 読み込みで捨てられる要素（名前・モニタ・範囲が不正、名前の重複）は、その場所にそのまま残す。
fn write_regions(root: &mut Table, regions: &[Region]) {
    let old = match root
        .get_mut("regions")
        .and_then(Item::as_array_of_tables_mut)
    {
        Some(tables) => std::mem::take(tables),
        None if regions.is_empty() => return,
        None => ArrayOfTables::new(),
    };
    let kept: Vec<(usize, String)> = read_regions(&old, &mut Reader { issues: Vec::new() })
        .into_iter()
        .map(|(i, region)| (i, region.name))
        .collect();

    // 定型領域ごとの要素の位置
    let mut slots: Vec<Option<usize>> = regions
        .iter()
        .map(|region| {
            kept.iter()
                .find(|(_, name)| *name == region.name)
                .map(|&(i, _)| i)
        })
        .collect();
    let mut free = kept
        .iter()
        .map(|&(i, _)| i)
        .filter(|i| !slots.contains(&Some(*i)))
        .collect::<Vec<_>>()
        .into_iter();
    for slot in slots.iter_mut().filter(|slot| slot.is_none()) {
        *slot = free.next();
    }

    let positions: Vec<Option<isize>> = old.iter().map(Table::position).collect();
    let mut old: Vec<Option<Table>> = old.into_iter().map(Some).collect();
    let mut queue = regions.iter().zip(slots.iter().copied());
    // 定型領域 `region` を書いた要素。`slot` の要素があればそれに書く。
    let take = |old: &mut Vec<Option<Table>>,
                (region, slot): (&Region, Option<usize>),
                position: Option<isize>| {
        let mut table = slot.and_then(|i| old[i].take()).unwrap_or_else(Table::new);
        table.set_position(position);
        write_region(&mut table, region);
        table
    };
    let mut new = ArrayOfTables::new();
    for i in 0..old.len() {
        if !kept.iter().any(|&(k, _)| k == i) {
            if let Some(table) = old[i].take() {
                new.push(table);
            }
            continue;
        }
        if !slots.contains(&Some(i)) {
            continue; // 対応する定型領域がなくなった
        }
        if let Some(next) = queue.next() {
            new.push(take(&mut old, next, positions[i]));
        }
    }
    // 位置を持たない要素は、直前の表の後ろに書かれる
    for next in queue {
        new.push(take(&mut old, next, None));
    }
    root.insert("regions", Item::ArrayOfTables(new));
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

fn rgb_str(&Rgb(r, g, b): &Rgb) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
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
    use crate::settings::load;

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
    fn a_changed_region_keeps_its_comments_and_order() {
        let out = written(TWO_REGIONS, |s| {
            s.regions[1].rect = Rect::new(100, 100, 130, 120);
            s.regions[1].scrap = true;
        });
        assert_eq!(
            out,
            TWO_REGIONS
                .replace("height = 20\n", "height = 20\nscrap = true\n")
                .replace("width = 20 # 幅", "width = 30 # 幅")
        );
    }

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
    fn a_removed_region_takes_only_its_own_lines() {
        let out = written(TWO_REGIONS, |s| {
            s.regions.remove(0);
        });
        let (read, issues) = load(&out);
        assert_eq!(issues, vec![]);
        assert_eq!(read.regions.len(), 1);
        assert_eq!(read.regions[0].name, "b");
        assert!(out.contains("# 右下\nname = \"b\""), "{out}");
        assert!(!out.contains("左上"), "{out}");
        assert!(out.ends_with("[scrap]\nshadow = false\n"), "{out}");
    }

    /// `TWO_REGIONS` の定型領域 `name` の行（`[[regions]]` の前のコメントから、次の空行の前まで）。
    fn region_lines(name: &str) -> &'static str {
        match name {
            "a" => {
                "# 定型領域\n[[regions]]\nname = \"a\"            # 左上\nmonitor = 'M1'\nx = 0\ny = 0\nwidth = 10\nheight = 10\n"
            }
            _ => {
                "[[regions]]\n# 右下\nname = \"b\"\nmonitor = 'M1'\nx = 100\ny = 100\nwidth = 20 # 幅\nheight = 20\n"
            }
        }
    }

    fn names(out: &str) -> Vec<String> {
        load(out).0.regions.into_iter().map(|r| r.name).collect()
    }

    #[test]
    fn reordered_regions_move_with_their_comments() {
        let out = written(TWO_REGIONS, |s| s.regions.reverse());
        assert_eq!(
            out,
            format!(
                "version = 1\n\n{}\n{}\n[scrap]\nshadow = false\n",
                region_lines("b"),
                region_lines("a")
            )
        );
        assert_eq!(names(&out), ["b", "a"]);
    }

    #[test]
    fn a_renamed_region_is_rewritten_in_place() {
        let out = written(TWO_REGIONS, |s| s.regions[0].name = "左上".to_owned());
        assert_eq!(out, TWO_REGIONS.replace("name = \"a\"", "name = \"左上\""));
    }

    #[test]
    fn a_region_inserted_between_others_keeps_the_order() {
        let out = written(TWO_REGIONS, |s| s.regions.insert(1, region("c")));
        assert_eq!(names(&out), ["a", "c", "b"]);
        assert!(out.contains(region_lines("a")), "{out}");
        assert!(out.contains(region_lines("b")), "{out}");
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
        let out = written(src, |s| {
            s.regions.remove(0);
        });
        assert_eq!(
            out,
            r#"version = 1
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
"#
        );
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
    fn a_missing_version_is_written() {
        let out = written("[scrap]\nshadow = false\n", |_| {});
        assert_eq!(load(&out).1, vec![]);
    }
}
