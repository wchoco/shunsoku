//! 設定 GUI の、入力で変わる状態（写しの中身ではないもの）と、出し方の計算。Win32 の層は通知を渡して、決まったことに従う。

use std::ops::RangeInclusive;
use std::str::FromStr;

use crate::geometry::{Rect, Size};

/// 定型領域の詳細の欄が指す領域。結び直したら、次に欄に出すときはフォーカスのある欄も書き換える。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegionDetail {
    index: usize,
    /// 結び直してから、まだ欄に出していない。
    rebound: bool,
}

impl RegionDetail {
    /// 領域 `index` に結び直す。
    pub fn bind(&mut self, index: usize) {
        self.index = index;
        self.rebound = true;
    }

    /// 指している領域。
    pub fn index(&self) -> usize {
        self.index
    }

    /// 欄に出し始める。この回にどの欄を書き換えるかを返す。
    pub fn begin_fill(&mut self) -> FillPass {
        FillPass {
            rebound: std::mem::take(&mut self.rebound),
        }
    }
}

/// 写しを欄に出す 1 回。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillPass {
    rebound: bool,
}

impl FillPass {
    /// テキストの欄を書き換えるか。`focused` はフォーカスがあるか、`region_field` は定型領域の詳細の欄か。
    /// 打っている途中の欄は書き換えない（末尾の空白などを消さない）。ただし結び直した直後の詳細の欄は、
    /// 前の領域の値が残らないよう書き換える。
    pub fn overwrites(self, focused: bool, region_field: bool) -> bool {
        !focused || (self.rebound && region_field)
    }
}

/// 数の欄の入力 `s` を読む。前後の空白は無視し、`range` の中の整数だけを読む（ほかは読めない入力）。
pub fn read_number<T>(s: &str, range: &RangeInclusive<T>) -> Option<T>
where
    T: FromStr + PartialOrd,
{
    s.trim().parse().ok().filter(|n| range.contains(n))
}

/// フォーカスのあるテキストの欄と、入ったときの文字。読めない値で出たら、入ったときの値に戻す（キーを打つ
/// たびに読める値は写しに入るので、打つ途中の値に戻さないように）。`K` は欄を指す値。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldEntry<K> {
    entered: Option<(K, String)>,
}

impl<K> Default for FieldEntry<K> {
    fn default() -> Self {
        FieldEntry { entered: None }
    }
}

impl<K: PartialEq> FieldEntry<K> {
    /// 欄 `field` に入った。`text` はそのときの欄の文字（写しの値）。
    pub fn entered(&mut self, field: K, text: impl Into<String>) {
        self.entered = Some((field, text.into()));
    }

    /// 欄 `field` から出た。`readable` は今の文字が読めるか。読めなければ、写しと欄に戻す文字（入ったときの
    /// 文字）を返す。入ったのと違う欄なら何も返さない。
    pub fn left(&mut self, field: K, readable: bool) -> Option<String> {
        let (entered, text) = self.entered.take()?;
        (entered == field && !readable).then_some(text)
    }

    /// 適用（OK・適用・Enter）の前に、フォーカスのある欄 `field` を、欄から出たときと同じに確かめる（Enter は
    /// フォーカスを動かさないので、出る前に適用が来る）。`text` は今の文字、`readable` はそれが読めるか。
    ///
    /// - 読めなければ、写しと欄に戻す文字（入ったときの文字）を返す。呼び出し側は適用しない。欄にいるままなので、
    ///   入ったときの文字を覚えたままにする。
    /// - 読めれば None。今の文字を入ったときの文字として覚え直す（適用した値が、次に戻す値になる）。
    /// - 入ったのと違う欄なら何もしない。
    pub fn applying(
        &mut self,
        field: K,
        text: impl Into<String>,
        readable: bool,
    ) -> Option<String> {
        let (entered, kept) = self.entered.as_mut()?;
        if *entered != field {
            return None;
        }
        if readable {
            *kept = text.into();
            None
        } else {
            Some(kept.clone())
        }
    }
}

/// 設定 GUI のキーの組み合わせの欄にフォーカスがある間、全体のホットキーの登録を外しておく（押したキーが
/// 欄に届くように）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HotkeyPause {
    paused: bool,
}

/// ホットキーの登録をどうするか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseChange {
    /// 登録を外す。
    Pause,
    /// 登録し直す。
    Resume,
}

impl HotkeyPause {
    /// 外している間か。この間は、設定が変わってもホットキーを登録しない。
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// フォーカスが変わった。`in_chord_field` はキーの組み合わせの欄にあるか。
    pub fn focus_changed(&mut self, in_chord_field: bool) -> Option<PauseChange> {
        match (in_chord_field, self.paused) {
            (true, false) => {
                self.paused = true;
                Some(PauseChange::Pause)
            }
            (false, true) => {
                self.paused = false;
                Some(PauseChange::Resume)
            }
            _ => None,
        }
    }

    /// 設定 GUI が閉じた。
    pub fn closed(&mut self) -> Option<PauseChange> {
        self.focus_changed(false)
    }

    /// 適用で、登録できるかを確かめるため登録し直す。欄にフォーカスが残っていれば、次のフォーカスの通知で外し直す。
    pub fn reregistering(&mut self) {
        self.paused = false;
    }
}

/// 設定 GUI で、コントロールが自分では受けないクリックの場所（ダイアログかタブに届く）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickPlace {
    /// ダイアログの地・ラベル・タブのページの地。
    Background,
    /// タブの見出し（タブが自分でフォーカスを取る）。
    TabHeader,
}

/// `place` をクリックしたら、キーの組み合わせの欄からフォーカスを外して、どの部品も選ばれていない状態にするか
/// （#118）。`in_chord_field` はいまフォーカスが欄にあるか。外れれば、`HotkeyPause` がホットキーを登録し直す。
pub fn click_blurs(in_chord_field: bool, place: ClickPlace) -> bool {
    in_chord_field && place == ClickPlace::Background
}

/// 定型領域の一覧の、範囲の絵（モニタの中のどこか）。座標は絵の左上からのピクセル。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thumbnail {
    /// モニタの枠。
    pub monitor: Rect,
    /// 範囲。モニタの外ならない。
    pub range: Option<Rect>,
}

/// 大きさ `monitor` のモニタの中の範囲 `range`（モニタ相対）を、大きさ `size` の絵にする。モニタは縦横比を保って
/// 真ん中に置く。小さな範囲も 1 px は見せ、モニタからはみ出す分は切る。モニタがわからなければ（見つからない）、
/// 原点から範囲の右下までをモニタとみなす。
pub fn range_thumbnail(monitor: Option<Size>, range: Rect, size: Size) -> Thumbnail {
    let monitor = monitor.unwrap_or(Size::new(
        range.right.max(1) as u32,
        range.bottom.max(1) as u32,
    ));
    let (mw, mh) = (
        f64::from(monitor.width.max(1)),
        f64::from(monitor.height.max(1)),
    );
    let scale = (f64::from(size.width) / mw).min(f64::from(size.height) / mh);
    let fw = (mw * scale).round() as i32;
    let fh = (mh * scale).round() as i32;
    let left = (size.width as i32 - fw) / 2;
    let top = (size.height as i32 - fh) / 2;
    let frame = Rect::new(left, top, left + fw, top + fh);
    let bounds = Rect::new(0, 0, monitor.width as i32, monitor.height as i32);
    let range = range.intersect(&bounds).map(|r| {
        let at = |v: i32| (f64::from(v) * scale).round() as i32;
        let (l, t) = (left + at(r.left), top + at(r.top));
        let right = (left + at(r.right)).max(l + 1).min(frame.right.max(l + 1));
        let bottom = (top + at(r.bottom)).max(t + 1).min(frame.bottom.max(t + 1));
        Rect::new(l, t, right, bottom)
    });
    Thumbnail {
        monitor: frame,
        range,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::editor::Editor;
    use crate::settings::{OPACITY_RANGE, Settings};

    #[test]
    fn after_binding_another_region_the_next_fill_overwrites_the_focused_region_field_once() {
        let mut detail = RegionDetail::default();
        detail.bind(2);
        assert_eq!(detail.index(), 2);

        let pass = detail.begin_fill();
        // 前の領域の値が残らないよう、フォーカスのある詳細の欄も書き換える
        assert!(pass.overwrites(true, true));
        // 全般の欄でフォーカスのあるものは、打っている途中なので書き換えない
        assert!(!pass.overwrites(true, false));
        assert!(pass.overwrites(false, true));
        assert!(pass.overwrites(false, false));

        // 次からは、打っている途中の詳細の欄も書き換えない（末尾の空白を消さない）
        let pass = detail.begin_fill();
        assert!(!pass.overwrites(true, true));
        assert!(pass.overwrites(false, true));
    }

    #[test]
    fn hotkeys_pause_while_a_chord_field_has_focus_and_resume_when_it_leaves_or_the_gui_closes() {
        let mut pause = HotkeyPause::default();
        assert!(!pause.is_paused());
        assert_eq!(pause.focus_changed(true), Some(PauseChange::Pause));
        assert!(pause.is_paused());
        // 欄から欄へ移っても外したまま
        assert_eq!(pause.focus_changed(true), None);
        assert_eq!(pause.focus_changed(false), Some(PauseChange::Resume));
        assert_eq!(pause.focus_changed(false), None);
        assert!(!pause.is_paused());

        assert_eq!(pause.closed(), None);
        pause.focus_changed(true);
        assert_eq!(pause.closed(), Some(PauseChange::Resume));
        assert!(!pause.is_paused());
    }

    #[test]
    fn clicking_the_background_takes_focus_off_a_chord_field_and_hotkeys_resume() {
        // 背景・ラベル・タブのページの地をクリックしたら外す
        assert!(click_blurs(true, ClickPlace::Background));
        let mut pause = HotkeyPause::default();
        pause.focus_changed(true);
        assert_eq!(pause.focus_changed(false), Some(PauseChange::Resume));
    }

    #[test]
    fn clicking_a_tab_or_with_focus_elsewhere_leaves_focus_as_it_is() {
        // タブの見出しはタブが自分でフォーカスを取る。ほかの欄にフォーカスがあれば今のまま
        assert!(!click_blurs(true, ClickPlace::TabHeader));
        assert!(!click_blurs(false, ClickPlace::Background));
        assert!(!click_blurs(false, ClickPlace::TabHeader));
    }

    #[test]
    fn applying_registers_again_and_a_field_still_focused_pauses_again() {
        let mut pause = HotkeyPause::default();
        pause.focus_changed(true);
        pause.reregistering();
        assert!(!pause.is_paused());
        assert_eq!(pause.focus_changed(true), Some(PauseChange::Pause));
    }

    /// Win32 層の不透明度の欄と同じく、読める入力だけを写しに入れる（`fields::set`）。
    fn type_opacity(editor: &mut Editor, s: &str) {
        if let Some(n) = read_opacity(s) {
            editor.draft_mut().scrap.opacity = n;
        }
    }

    fn read_opacity(s: &str) -> Option<u8> {
        read_number(s, &OPACITY_RANGE)
    }

    #[test]
    fn a_number_field_reads_whole_numbers_in_its_range() {
        assert_eq!(read_number(" 50 ", &OPACITY_RANGE), Some(50));
        assert_eq!(read_number("10", &OPACITY_RANGE), Some(10));
        assert_eq!(read_number("100", &OPACITY_RANGE), Some(100));
        // 範囲の外、空、数でないものは読めない
        assert_eq!(read_number("5", &OPACITY_RANGE), None);
        assert_eq!(read_number("500", &OPACITY_RANGE), None);
        assert_eq!(read_number("", &OPACITY_RANGE), None);
        assert_eq!(read_number("-20", &OPACITY_RANGE), None);
        assert_eq!(read_number("50%", &OPACITY_RANGE), None);
        assert_eq!(read_number("70000", &(1..=u32::MAX)), Some(70000));
    }

    #[test]
    fn leaving_a_field_with_an_unreadable_value_restores_the_value_it_had_when_entered() {
        let mut editor = Editor::new(Settings::default());
        let mut entry = FieldEntry::default();
        entry.entered("opacity", "100");
        // 100 → 500 と打つ途中の 50 は読めるので写しに入る
        for typed in ["10", "1", "", "5", "50", "500"] {
            type_opacity(&mut editor, typed);
        }
        assert_eq!(editor.draft().scrap.opacity, 50);

        // 打つ途中の 50 ではなく、入ったときの 100 に戻す
        let restore = entry.left("opacity", read_opacity("500").is_some());
        assert_eq!(restore.as_deref(), Some("100"));
        type_opacity(&mut editor, restore.as_deref().unwrap_or_default());
        assert_eq!(editor.draft(), editor.base());
        assert!(!editor.buttons().apply);
    }

    #[test]
    fn leaving_a_field_with_a_readable_value_keeps_it() {
        let mut entry = FieldEntry::default();
        entry.entered("opacity", "100");
        assert_eq!(entry.left("opacity", true), None);
        // 出たあとは覚えていない
        assert_eq!(entry.left("opacity", false), None);
    }

    #[test]
    fn leaving_a_field_other_than_the_one_entered_restores_nothing() {
        let mut entry = FieldEntry::default();
        entry.entered("save_dir", r"C:\shots");
        assert_eq!(entry.left("opacity", false), None);

        // 別の欄に入り直したら、その欄の値を覚える
        entry.entered("save_dir", r"C:\shots");
        entry.entered("opacity", "80");
        assert_eq!(entry.left("opacity", false).as_deref(), Some("80"));
    }

    #[test]
    fn applying_with_an_unreadable_value_in_the_focused_field_restores_it_and_stays() {
        // Enter（既定の OK）はフォーカスを動かさないので、欄から出る前に適用が来る
        let mut editor = Editor::new(Settings::default());
        let mut entry = FieldEntry::default();
        entry.entered("opacity", "100");
        for typed in ["10", "1", "", "5", "50", "500"] {
            type_opacity(&mut editor, typed);
        }

        // 打つ途中の 50 を書かず、入ったときの 100 に戻して適用しない
        let restore = entry.applying("opacity", "500", false);
        assert_eq!(restore.as_deref(), Some("100"));
        type_opacity(&mut editor, restore.as_deref().unwrap_or_default());
        assert_eq!(editor.draft(), editor.base());

        // 欄にいるままなので、また読めない値で出たら同じ値に戻す
        assert_eq!(entry.left("opacity", false).as_deref(), Some("100"));
    }

    #[test]
    fn applying_with_a_readable_value_remembers_it_as_the_value_to_return_to() {
        let mut entry = FieldEntry::default();
        entry.entered("opacity", "100");
        assert_eq!(entry.applying("opacity", "80", true), None);
        // 適用したあとに読めない値で出たら、適用した値に戻す
        assert_eq!(entry.left("opacity", false).as_deref(), Some("80"));
    }

    #[test]
    fn applying_with_focus_outside_the_entered_field_restores_nothing() {
        let mut entry = FieldEntry::default();
        assert_eq!(entry.applying("opacity", "500", false), None);
        entry.entered("save_dir", r"C:\shots");
        assert_eq!(entry.applying("opacity", "500", false), None);
    }

    #[test]
    fn the_thumbnail_fits_the_monitor_in_the_box_and_scales_the_range_with_it() {
        // 1920×1080 を 32×20 に: 縦横比を保って 32×18、上下に 1 px ずつ余らせる
        let t = range_thumbnail(
            Some(Size::new(1920, 1080)),
            Rect::new(960, 540, 1920, 1080),
            Size::new(32, 20),
        );
        assert_eq!(t.monitor, Rect::new(0, 1, 32, 19));
        assert_eq!(t.range, Some(Rect::new(16, 10, 32, 19)));
    }

    #[test]
    fn a_tiny_range_still_shows_as_one_pixel_and_one_outside_the_monitor_is_clipped() {
        let t = range_thumbnail(
            Some(Size::new(1000, 1000)),
            Rect::new(0, 0, 2, 2),
            Size::new(20, 20),
        );
        assert_eq!(t.range, Some(Rect::new(0, 0, 1, 1)));

        let t = range_thumbnail(
            Some(Size::new(1000, 1000)),
            Rect::new(500, 500, 3000, 3000),
            Size::new(20, 20),
        );
        assert_eq!(t.range, Some(Rect::new(10, 10, 20, 20)));

        let t = range_thumbnail(
            Some(Size::new(1000, 1000)),
            Rect::new(1200, 0, 1300, 100),
            Size::new(20, 20),
        );
        assert_eq!(t.range, None);
    }

    #[test]
    fn without_its_monitor_the_thumbnail_is_scaled_to_the_ranges_bottom_right() {
        let t = range_thumbnail(None, Rect::new(100, 0, 200, 100), Size::new(20, 20));
        assert_eq!(t.monitor, Rect::new(0, 5, 20, 15));
        assert_eq!(t.range, Some(Rect::new(10, 5, 20, 15)));
    }
}
