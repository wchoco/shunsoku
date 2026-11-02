//! 範囲選択の画面の状態機械（通常モードと精密モード）。
//!
//! - モード（`Mode`）は `Tab`（`Input::ToggleMode`）で切り替える。切り替えはその `Selection` の間だけで、
//!   開くたびに設定の既定のモード（`with_mode`）から始まる。
//! - 通常モードでは、範囲を決めた時点（ドラッグを離す、`Space`、`Ctrl+A`）で確定する。精密モードでは
//!   確定せずに調整の状態（`State::Adjusting`）に入り、つまみ・範囲の中のドラッグ・矢印で直してから
//!   `Enter`・ダブルクリックで確定する。
//! - 画面に描くもの（範囲、つまみ、十字線、カーソルの横の座標、拡大鏡）は `view` にまとめる。Win32 層は
//!   入力の前後で `view` を比べ、変わったときだけ描き直す。
//! - 入力（`Input`）は Win32 層がマウスとキーのイベントから作る。座標はすべてスクリーン座標（物理ピクセル）。
//! - モニタの一覧（`MonitorInfo`）は Win32 層が集めて渡す。
//! - 範囲は、始点（押した画素、または `Space` を押したときのカーソルの画素）とカーソルの画素の両方を含む矩形。
//! - キーボードの操作（矢印・`Space`・`Ctrl+A`・`Ctrl+Shift+A`）もここで状態に変える。Win32 層はキーを
//!   `Input` にして渡し、矢印で動いた `cursor` に実際のマウスのカーソルを合わせるだけにする。
//! - 矢印で動かすカーソルは、どれかのモニタの画素に留める（隙間や画面の外には出ない）。
//! - ウィンドウの一覧（`WindowInfo`）も Win32 層が集めて渡す。`Shift` を押している間はカーソルの下の
//!   ウィンドウ（`Ctrl` も押していれば部品）をハイライトし、クリックか `Enter` でそれを選ぶ（`chosen_window`）。

use crate::geometry::{Point, Rect};

mod highlight;

pub use highlight::{Highlight, WindowId, WindowInfo, WindowKind};

/// 1 枚のモニタ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorInfo {
    /// スクリーン座標（物理ピクセル）での位置と大きさ。
    pub rect: Rect,
    /// 実効 DPI（100% で 96）。
    pub dpi: u32,
}

/// マウスのボタン。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

/// 矢印キーの向き。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// 範囲選択の画面のモード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// ドラッグを離した時点で範囲が決まる。
    #[default]
    Normal,
    /// 範囲を調整してから確定する。十字線とカーソルの横の座標を常に出す。
    Precise,
}

/// 調整の状態で、範囲の大きさを変えるつまみ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Handle {
    /// すべて。角を先に並べる（重なったら角を選ぶ）。
    pub const ALL: [Handle; 8] = [
        Handle::TopLeft,
        Handle::TopRight,
        Handle::BottomRight,
        Handle::BottomLeft,
        Handle::Top,
        Handle::Right,
        Handle::Bottom,
        Handle::Left,
    ];

    /// 動かす辺（左、上、右、下）。
    const fn edges(self) -> (bool, bool, bool, bool) {
        match self {
            Handle::TopLeft => (true, true, false, false),
            Handle::Top => (false, true, false, false),
            Handle::TopRight => (false, true, true, false),
            Handle::Right => (false, false, true, false),
            Handle::BottomRight => (false, false, true, true),
            Handle::Bottom => (false, false, false, true),
            Handle::BottomLeft => (true, false, false, true),
            Handle::Left => (true, false, false, false),
        }
    }

    /// つまみの中心（範囲の枠の上の点）。右と下は範囲に含まない側の座標。
    fn center(self, r: Rect) -> Point {
        let cx = r.left + (r.right - r.left) / 2;
        let cy = r.top + (r.bottom - r.top) / 2;
        let x = match self {
            Handle::TopLeft | Handle::BottomLeft | Handle::Left => r.left,
            Handle::Top | Handle::Bottom => cx,
            Handle::TopRight | Handle::Right | Handle::BottomRight => r.right,
        };
        let y = match self {
            Handle::TopLeft | Handle::Top | Handle::TopRight => r.top,
            Handle::Left | Handle::Right => cy,
            Handle::BottomLeft | Handle::Bottom | Handle::BottomRight => r.bottom,
        };
        Point::new(x, y)
    }
}

/// カーソルの形。Win32 層が `WM_SETCURSOR` で当てる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    /// 十字（既定）。
    Cross,
    /// 範囲の中（ドラッグで動かす）。
    Move,
    /// 左上・右下のつまみ。
    ResizeNwse,
    /// 右上・左下のつまみ。
    ResizeNesw,
    /// 上・下のつまみ。
    ResizeNs,
    /// 左・右のつまみ。
    ResizeWe,
}

/// つまみの一辺（96 DPI での物理ピクセル。モニタの DPI で換算する）。
const HANDLE_SIDE: i32 = 7;
/// つまみをつかめる、見た目より外側の幅（96 DPI での物理ピクセル）。
const HANDLE_GRIP: i32 = 3;

/// 矢印でカーソルを動かす量（物理ピクセル）。
const ARROW_STEP: i32 = 1;
/// Shift+矢印でカーソルを動かす量（物理ピクセル）。
const SHIFT_ARROW_STEP: i32 = 10;

/// 範囲選択の画面への入力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Press {
        button: Button,
        at: Point,
    },
    Move {
        at: Point,
    },
    Release {
        button: Button,
        at: Point,
    },
    /// `Esc`。1 段戻る。
    Escape,
    /// 矢印（`shift` なら Shift+矢印）。カーソルを動かす。調整の状態では範囲を動かす。
    Arrow {
        direction: Direction,
        shift: bool,
    },
    /// Ctrl+矢印（`shift` なら Ctrl+Shift+矢印）。調整の状態では範囲の右下の辺を動かす。
    /// ほかの状態では矢印と同じ。
    ResizeArrow {
        direction: Direction,
        shift: bool,
    },
    /// `Space`。何も選んでいなければカーソルの画素を始点にし、選んでいれば範囲を決める。
    Space,
    /// `Enter`。調整の状態なら確定する。ハイライトしていれば、そのウィンドウを撮るものに決める。
    Enter,
    /// 左ボタンのダブルクリック（2 回目の押下の代わりに来る）。調整の状態で範囲の中なら確定する。
    /// ほかは左ボタンの押下と同じ。
    DoubleClick {
        at: Point,
    },
    /// `Tab`。通常モードと精密モードを切り替える。
    ToggleMode,
    /// `Ctrl+A`。カーソルのあるモニタ全体を選んで確定する。
    SelectMonitor,
    /// `Ctrl+Shift+A`。全モニタを囲む矩形を選んで確定する（モニタの隙間は撮るときに透明になる）。
    SelectAllMonitors,
    /// Shift と Ctrl の押し下げの状態が変わった。Shift でウィンドウを、Shift+Ctrl で部品をハイライトする。
    Modifiers {
        shift: bool,
        ctrl: bool,
    },
}

/// 状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// 何も選んでいない。
    Idle,
    /// 範囲を選んでいる。
    Selecting,
    /// 精密モードで範囲を決め、調整している（`Selection::rect`）。
    Adjusting,
    /// 範囲が決まった（`Selection::rect`）。
    Confirmed,
    /// 中止した。
    Cancelled,
}

/// 範囲選択の状態。
#[derive(Debug, Clone)]
pub struct Selection {
    monitors: Vec<MonitorInfo>,
    state: State,
    /// 範囲の始点。`Selecting` と `Confirmed` のときだけ Some。
    anchor: Option<Point>,
    /// カーソルの画素。キーボードで動かす操作もこれを動かす。
    cursor: Point,
    /// ウィンドウのハイライト（`highlight`）。
    windows: Windows,
    mode: Mode,
    /// 通常モードでも十字線を出す（設定）。
    crosshair: bool,
    /// 調整している範囲。`Adjusting` のときだけ Some。
    adjusted: Option<Rect>,
    /// 調整の状態でつかんでいるもの。左ボタンを押している間だけ Some。
    grab: Option<Grab>,
}

/// ウィンドウのハイライトに使う状態。
#[derive(Debug, Clone, Default)]
struct Windows {
    /// 開いた時点のウィンドウの一覧（Z オーダーの手前から奥）。
    list: Vec<WindowInfo>,
    shift: bool,
    ctrl: bool,
    /// ハイライトから撮るウィンドウに決まった（`State::Confirmed`）。
    chosen: Option<Highlight>,
}

/// 調整の状態でつかんだもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Grab {
    /// つまみ。None なら範囲の中（位置を動かす）。
    handle: Option<Handle>,
    /// 押した点。
    from: Point,
    /// 押したときの範囲。
    start: Rect,
}

/// 範囲選択の画面に描くもの。Win32 層は入力の前後でこれを比べ、変わったときだけ描き直す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// 範囲。何も選んでいなければ None。
    pub rect: Option<Rect>,
    /// 範囲の左上の外側に出すサイズ（`Selection::size_label`）。
    pub size_label: Option<String>,
    /// 調整の状態のつまみ（スクリーン座標）。ほかの状態では空。
    pub handles: Vec<Rect>,
    /// 十字線を通すカーソルの画素。出さなければ None。
    pub crosshair: Option<Point>,
    /// カーソルの横に出す文字と、そのカーソルの画素（精密モードだけ）。
    pub cursor_label: Option<(Point, String)>,
    /// ハイライトしているウィンドウ（`Selection::highlight`）。このときは範囲とサイズを出さない。
    pub highlight: Option<Highlight>,
    /// 拡大鏡で見せるカーソルの画素（精密モードだけ。置き場所は `magnifier::place`）。
    pub magnifier: Option<Point>,
}

impl Selection {
    /// `cursor` は開いた時点のカーソルの位置。
    pub fn new(monitors: Vec<MonitorInfo>, cursor: Point) -> Selection {
        Selection {
            monitors,
            state: State::Idle,
            anchor: None,
            cursor,
            windows: Windows::default(),
            mode: Mode::Normal,
            crosshair: false,
            adjusted: None,
            grab: None,
        }
    }

    /// ハイライトの対象にするウィンドウの一覧を渡す（Z オーダーの手前から奥）。
    pub fn set_windows(&mut self, windows: Vec<WindowInfo>) {
        self.windows.list = windows;
    }

    /// いまハイライトしているもの。Shift を押していて、範囲を選んでいないとき（押しただけでまだ
    /// 動かしていないときを含む）だけ。
    pub fn highlight(&self) -> Option<Highlight> {
        let unmoved = match self.state {
            State::Idle => true,
            State::Selecting => self.anchor == Some(self.cursor),
            State::Adjusting | State::Confirmed | State::Cancelled => false,
        };
        if !self.windows.shift || !unmoved {
            return None;
        }
        highlight::find(&self.windows.list, self.cursor, self.windows.ctrl)
    }

    /// ハイライトから撮ることに決まったウィンドウ。範囲で確定したときや、まだ決まっていなければ None。
    pub fn chosen_window(&self) -> Option<&Highlight> {
        self.windows.chosen.as_ref()
    }

    /// ハイライトしていたものを撮ることにして確定する。何もハイライトしていなければ何もしない。
    fn choose_window(&mut self, highlight: Option<Highlight>) {
        if let Some(highlight) = highlight {
            self.windows.chosen = Some(highlight);
            self.anchor = None;
            self.state = State::Confirmed;
        }
    }

    /// 開いたときのモード（設定の既定のモード）。
    pub fn with_mode(mut self, mode: Mode) -> Selection {
        self.mode = mode;
        self
    }

    /// 通常モードでも十字線を出す（設定）。精密モードでは設定によらず出す。
    pub fn with_crosshair(mut self, on: bool) -> Selection {
        self.crosshair = on;
        self
    }

    /// 入力を 1 つ受けて、受けたあとの状態を返す。確定・中止のあとの入力は無視する。
    pub fn handle(&mut self, input: Input) -> State {
        if matches!(self.state, State::Confirmed | State::Cancelled) {
            return self.state;
        }
        if self.state == State::Adjusting {
            self.adjust(input);
            return self.state;
        }
        match input {
            Input::Press {
                button: Button::Left,
                at,
            }
            | Input::DoubleClick { at } => {
                self.cursor = at;
                if self.state == State::Idle {
                    self.anchor = Some(at);
                    self.state = State::Selecting;
                }
            }
            Input::Move { at } => self.cursor = at,
            Input::Release {
                button: Button::Left,
                at,
            } => {
                self.cursor = at;
                if self.state == State::Selecting {
                    if self.anchor == Some(at) {
                        // 動かさずに離した（クリック）。ハイライトしていればそのウィンドウを撮り、
                        // していなければ何も選ばなかったことにする
                        let highlight = self.highlight();
                        self.reset();
                        self.choose_window(highlight);
                    } else {
                        self.decide();
                    }
                }
            }
            Input::Press {
                button: Button::Right,
                ..
            }
            | Input::Escape => self.back(),
            Input::Release {
                button: Button::Right,
                ..
            } => {}
            Input::Arrow { direction, shift } | Input::ResizeArrow { direction, shift } => {
                self.move_cursor(direction, shift)
            }
            Input::Space => match self.state {
                State::Idle => {
                    self.anchor = Some(self.cursor);
                    self.state = State::Selecting;
                }
                // 始点と同じ画素でも、キーで 2 度押したなら 1 画素の範囲として決める
                _ => self.decide(),
            },
            Input::ToggleMode => self.toggle_mode(),
            Input::SelectMonitor => {
                if let Some(rect) = self.monitor_at(self.cursor).map(|m| m.rect) {
                    self.settle(rect);
                }
            }
            Input::SelectAllMonitors => {
                let all = self.monitors.iter().map(|m| m.rect).reduce(|a, b| {
                    Rect::new(
                        a.left.min(b.left),
                        a.top.min(b.top),
                        a.right.max(b.right),
                        a.bottom.max(b.bottom),
                    )
                });
                if let Some(rect) = all {
                    self.settle(rect);
                }
            }
            Input::Modifiers { shift, ctrl } => {
                self.windows.shift = shift;
                self.windows.ctrl = ctrl;
            }
            Input::Enter => self.choose_window(self.highlight()),
        }
        self.state
    }

    /// 1 段戻る。選んでいる・調整していれば選び直し、何も選んでいなければ中止。
    fn back(&mut self) {
        match self.state {
            State::Selecting | State::Adjusting => self.reset(),
            State::Idle => self.state = State::Cancelled,
            State::Confirmed | State::Cancelled => {}
        }
    }

    fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            Mode::Normal => Mode::Precise,
            Mode::Precise => Mode::Normal,
        };
    }

    /// 選んでいる範囲（始点とカーソル）を決める。通常モードなら確定し、精密モードなら調整に入る。
    fn decide(&mut self) {
        if let Some(rect) = self.rect() {
            self.settle(rect);
        }
    }

    /// `rect` を範囲に決める。通常モードなら確定し、精密モードなら調整に入る。
    fn settle(&mut self, rect: Rect) {
        match self.mode {
            Mode::Normal => self.confirm(rect),
            Mode::Precise => {
                self.anchor = None;
                self.adjusted = Some(rect);
                self.grab = None;
                self.state = State::Adjusting;
            }
        }
    }

    /// 調整の状態での入力。
    fn adjust(&mut self, input: Input) {
        let Some(rect) = self.adjusted else {
            return;
        };
        match input {
            Input::Press {
                button: Button::Left,
                at,
            } => {
                self.cursor = at;
                match self.hit(rect, at) {
                    Some(handle) => {
                        self.grab = Some(Grab {
                            handle,
                            from: at,
                            start: rect,
                        })
                    }
                    None => self.reselect(at),
                }
            }
            Input::DoubleClick { at } => {
                self.cursor = at;
                match self.hit(rect, at) {
                    Some(_) => self.confirm(rect),
                    None => self.reselect(at),
                }
            }
            Input::Move { at } => {
                self.cursor = at;
                if let Some(grab) = self.grab {
                    self.adjusted = Some(self.dragged(grab, at));
                }
            }
            Input::Release {
                button: Button::Left,
                at,
            } => {
                self.cursor = at;
                if let Some(grab) = self.grab.take() {
                    self.adjusted = Some(self.dragged(grab, at));
                }
            }
            Input::Press {
                button: Button::Right,
                ..
            }
            | Input::Escape => self.back(),
            Input::Release {
                button: Button::Right,
                ..
            } => {}
            Input::Arrow { direction, shift } => {
                let (dx, dy) = step(direction, shift);
                self.adjusted = Some(self.keep_inside(rect.offset(dx, dy)));
            }
            Input::ResizeArrow { direction, shift } => {
                let (dx, dy) = step(direction, shift);
                let b = self.bounds();
                let right = (rect.right + dx).clamp(rect.left + 1, b.right.max(rect.left + 1));
                let bottom = (rect.bottom + dy).clamp(rect.top + 1, b.bottom.max(rect.top + 1));
                self.adjusted = Some(Rect::new(rect.left, rect.top, right, bottom));
            }
            Input::Enter | Input::Space => self.confirm(rect),
            Input::ToggleMode => self.toggle_mode(),
            Input::SelectMonitor | Input::SelectAllMonitors => {}
            Input::Modifiers { shift, ctrl } => {
                self.windows.shift = shift;
                self.windows.ctrl = ctrl;
            }
        }
    }

    /// 調整の状態で範囲の外を押した。そこを始点に選び直す。
    fn reselect(&mut self, at: Point) {
        self.adjusted = None;
        self.grab = None;
        self.anchor = Some(at);
        self.state = State::Selecting;
    }

    /// 調整の状態で `at` にあるもの。つまみなら Some(Some)、範囲の中なら Some(None)、外なら None。
    fn hit(&self, rect: Rect, at: Point) -> Option<Option<Handle>> {
        let found = Handle::ALL.into_iter().find(|&handle| {
            let grip = self.scaled(HANDLE_GRIP, handle.center(rect));
            self.handle_rect(rect, handle).inflate(grip).contains(at)
        });
        match found {
            Some(handle) => Some(Some(handle)),
            None => rect.contains(at).then_some(None),
        }
    }

    /// つまみの見た目の矩形。中心のあるモニタの DPI で大きさを換算する。
    fn handle_rect(&self, rect: Rect, handle: Handle) -> Rect {
        let c = handle.center(rect);
        let side = self.scaled(HANDLE_SIDE, c);
        let left = c.x - side / 2;
        let top = c.y - side / 2;
        Rect::new(left, top, left + side, top + side)
    }

    /// 96 DPI での長さ `len` を、`at` のあるモニタ（なければ 96 DPI）の物理ピクセルにする。
    fn scaled(&self, len: i32, at: Point) -> i32 {
        let dpi = self.monitor_at(at).map_or(96, |m| m.dpi) as i32;
        (len * dpi + 48) / 96
    }

    /// つかんだものを `at` まで引いたあとの範囲。大きさは 1 画素より小さくせず、全モニタを囲む矩形の
    /// 外には出さない。
    fn dragged(&self, grab: Grab, at: Point) -> Rect {
        let (dx, dy) = (at.x - grab.from.x, at.y - grab.from.y);
        let r = grab.start;
        let Some(handle) = grab.handle else {
            return self.keep_inside(r.offset(dx, dy));
        };
        let b = self.bounds();
        let (left, top, right, bottom) = handle.edges();
        let mut out = r;
        if left {
            out.left = (r.left + dx).clamp(b.left.min(r.right - 1), r.right - 1);
        }
        if top {
            out.top = (r.top + dy).clamp(b.top.min(r.bottom - 1), r.bottom - 1);
        }
        if right {
            out.right = (r.right + dx).clamp(r.left + 1, b.right.max(r.left + 1));
        }
        if bottom {
            out.bottom = (r.bottom + dy).clamp(r.top + 1, b.bottom.max(r.top + 1));
        }
        out
    }

    /// 全モニタを囲む矩形。
    fn bounds(&self) -> Rect {
        self.monitors
            .iter()
            .map(|m| m.rect)
            .reduce(|a, b| {
                Rect::new(
                    a.left.min(b.left),
                    a.top.min(b.top),
                    a.right.max(b.right),
                    a.bottom.max(b.bottom),
                )
            })
            .unwrap_or_default()
    }

    /// 大きさを変えずに、`rect` を全モニタを囲む矩形の中に戻す。
    fn keep_inside(&self, rect: Rect) -> Rect {
        let b = self.bounds();
        let dx = if rect.left < b.left {
            b.left - rect.left
        } else if rect.right > b.right {
            (b.right - rect.right).max(b.left - rect.left)
        } else {
            0
        };
        let dy = if rect.top < b.top {
            b.top - rect.top
        } else if rect.bottom > b.bottom {
            (b.bottom - rect.bottom).max(b.top - rect.top)
        } else {
            0
        };
        rect.offset(dx, dy)
    }

    /// 矢印でカーソルを動かす。
    fn move_cursor(&mut self, direction: Direction, shift: bool) {
        let (dx, dy) = step(direction, shift);
        let to = Point::new(self.cursor.x + dx, self.cursor.y + dy);
        if self.monitor_at(to).is_some() {
            self.cursor = to;
        } else if let Some(monitor) = self.monitor_at(self.cursor) {
            // 隙間や画面の外には出ず、いまのモニタの端で止まる
            let r = monitor.rect;
            self.cursor = Point::new(
                to.x.clamp(r.left, r.right - 1),
                to.y.clamp(r.top, r.bottom - 1),
            );
        }
    }

    /// `rect` を範囲にして確定する。
    fn confirm(&mut self, rect: Rect) {
        self.adjusted = None;
        self.grab = None;
        self.anchor = Some(rect.origin());
        self.cursor = Point::new(rect.right - 1, rect.bottom - 1);
        self.state = State::Confirmed;
    }

    /// `at` の画素を含むモニタ。
    fn monitor_at(&self, at: Point) -> Option<&MonitorInfo> {
        self.monitors.iter().find(|m| m.rect.contains(at))
    }

    /// 何も選んでいない状態に戻す（選び直し）。
    fn reset(&mut self) {
        self.anchor = None;
        self.adjusted = None;
        self.grab = None;
        self.state = State::Idle;
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// 選んでいる（調整している、決まった）範囲。何も選んでいなければ None。
    pub fn rect(&self) -> Option<Rect> {
        if let Some(rect) = self.adjusted {
            return Some(rect);
        }
        let a = self.anchor?;
        let c = self.cursor;
        Some(Rect::new(
            a.x.min(c.x),
            a.y.min(c.y),
            a.x.max(c.x) + 1,
            a.y.max(c.y) + 1,
        ))
    }

    /// カーソルの画素。
    pub fn cursor(&self) -> Point {
        self.cursor
    }

    pub fn monitors(&self) -> &[MonitorInfo] {
        &self.monitors
    }

    /// 調整の状態のつまみ（スクリーン座標）。ほかの状態では空。
    pub fn handles(&self) -> Vec<Rect> {
        match (self.state, self.adjusted) {
            (State::Adjusting, Some(rect)) => Handle::ALL
                .into_iter()
                .map(|handle| self.handle_rect(rect, handle))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// カーソルの形。調整の状態で、つまみの上なら大きさを変える形、範囲の中なら動かす形。
    /// つかんでいる間は、つかんだものの形のまま。
    pub fn cursor_shape(&self) -> CursorShape {
        let (State::Adjusting, Some(rect)) = (self.state, self.adjusted) else {
            return CursorShape::Cross;
        };
        let target = match self.grab {
            Some(grab) => Some(grab.handle),
            None => self.hit(rect, self.cursor),
        };
        match target {
            None => CursorShape::Cross,
            Some(None) => CursorShape::Move,
            Some(Some(Handle::TopLeft | Handle::BottomRight)) => CursorShape::ResizeNwse,
            Some(Some(Handle::TopRight | Handle::BottomLeft)) => CursorShape::ResizeNesw,
            Some(Some(Handle::Top | Handle::Bottom)) => CursorShape::ResizeNs,
            Some(Some(Handle::Left | Handle::Right)) => CursorShape::ResizeWe,
        }
    }

    /// 十字線を出すか。精密モードでは常に、通常モードでは設定（`with_crosshair`）に従う。
    pub fn shows_crosshair(&self) -> bool {
        self.mode == Mode::Precise || self.crosshair
    }

    /// カーソルの横に出す文字。精密モードだけ。カーソルの物理ピクセルの座標（`1234, 567`）と、
    /// ドラッグ中（範囲を選んでいる、つまみや範囲をつかんでいる）は範囲のサイズを足す。
    pub fn cursor_label(&self) -> Option<String> {
        if self.mode != Mode::Precise {
            return None;
        }
        let at = format!("{}, {}", self.cursor.x, self.cursor.y);
        let dragging = self.state == State::Selecting || self.grab.is_some();
        match self.rect().filter(|_| dragging) {
            Some(rect) => Some(format!("{at}   {}", size_text(rect))),
            None => Some(at),
        }
    }

    /// 画面に描くもの。
    pub fn view(&self) -> View {
        let highlight = self.highlight();
        let ranged = highlight.is_none();
        View {
            rect: self.rect().filter(|_| ranged),
            size_label: self.size_label().filter(|_| ranged),
            handles: self.handles(),
            crosshair: self.shows_crosshair().then_some(self.cursor),
            cursor_label: self.cursor_label().map(|text| (self.cursor, text)),
            highlight,
            magnifier: (self.mode == Mode::Precise).then_some(self.cursor),
        }
    }

    /// 範囲のモニタごとの部分。何も選んでいなければ空。
    pub fn parts(&self) -> Vec<Part> {
        self.rect()
            .map_or_else(Vec::new, |rect| split(rect, &self.monitors))
    }

    /// 範囲の左上の外側に出すサイズ（`1280 × 720`）。モニタをまたいでいれば、モニタごとのサイズを
    /// 左から `＋` でつなぐ。何も選んでいなければ None。
    pub fn size_label(&self) -> Option<String> {
        let rect = self.rect()?;
        let parts = split(rect, &self.monitors);
        let label = if parts.len() <= 1 {
            size_text(rect)
        } else {
            parts
                .iter()
                .map(|part| size_text(part.rect))
                .collect::<Vec<_>>()
                .join(" ＋ ")
        };
        Some(label)
    }
}

/// 矢印 1 回で動かす量。
fn step(direction: Direction, shift: bool) -> (i32, i32) {
    let step = if shift { SHIFT_ARROW_STEP } else { ARROW_STEP };
    match direction {
        Direction::Left => (-step, 0),
        Direction::Right => (step, 0),
        Direction::Up => (0, -step),
        Direction::Down => (0, step),
    }
}

fn size_text(rect: Rect) -> String {
    format!("{} × {}", rect.width(), rect.height())
}

/// 範囲のうち、1 枚のモニタに重なる部分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    /// `MonitorInfo` の一覧での番号。
    pub monitor: usize,
    /// 重なる部分（スクリーン座標）。
    pub rect: Rect,
}

/// `rect` をモニタごとの部分に分ける。どのモニタにも重ならない部分は含めない。
/// 並びは左から右（同じ左端なら上から下）。
pub fn split(rect: Rect, monitors: &[MonitorInfo]) -> Vec<Part> {
    let mut parts: Vec<Part> = monitors
        .iter()
        .enumerate()
        .filter_map(|(monitor, info)| {
            info.rect
                .intersect(&rect)
                .map(|rect| Part { monitor, rect })
        })
        .collect();
    parts.sort_by_key(|part| (part.rect.left, part.rect.top));
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: i32, y: i32) -> Point {
        Point::new(x, y)
    }

    /// 1920×1080 @100% の 1 枚。
    fn one_monitor() -> Selection {
        Selection::new(
            vec![MonitorInfo {
                rect: Rect::new(0, 0, 1920, 1080),
                dpi: 96,
            }],
            p(0, 0),
        )
    }

    #[test]
    fn dragging_selects_both_end_pixels_and_releasing_confirms() {
        let mut s = one_monitor();
        assert_eq!(s.state(), State::Idle);
        s.handle(Input::Press {
            button: Button::Left,
            at: p(10, 20),
        });
        s.handle(Input::Move { at: p(109, 69) });
        assert_eq!(s.state(), State::Selecting);
        assert_eq!(s.rect(), Some(Rect::new(10, 20, 110, 70)));
        let state = s.handle(Input::Release {
            button: Button::Left,
            at: p(109, 69),
        });
        assert_eq!(state, State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(10, 20, 110, 70)));
    }

    #[test]
    fn a_click_without_dragging_does_nothing() {
        let mut s = one_monitor();
        s.handle(Input::Press {
            button: Button::Left,
            at: p(10, 20),
        });
        let state = s.handle(Input::Release {
            button: Button::Left,
            at: p(10, 20),
        });
        assert_eq!(state, State::Idle);
        assert_eq!(s.rect(), None);
    }

    fn start_drag(s: &mut Selection) {
        s.handle(Input::Press {
            button: Button::Left,
            at: p(10, 20),
        });
        s.handle(Input::Move { at: p(300, 200) });
    }

    #[test]
    fn escape_while_selecting_starts_over_then_escape_again_cancels() {
        let mut s = one_monitor();
        start_drag(&mut s);
        assert_eq!(s.handle(Input::Escape), State::Idle);
        assert_eq!(s.rect(), None);
        // 押したままのボタンを離しても、選び直しのまま
        assert_eq!(
            s.handle(Input::Release {
                button: Button::Left,
                at: p(300, 200),
            }),
            State::Idle
        );
        assert_eq!(s.handle(Input::Escape), State::Cancelled);
    }

    #[test]
    fn right_click_while_selecting_starts_over_then_right_click_again_cancels() {
        let mut s = one_monitor();
        start_drag(&mut s);
        let right = Input::Press {
            button: Button::Right,
            at: p(300, 200),
        };
        assert_eq!(s.handle(right), State::Idle);
        assert_eq!(s.rect(), None);
        assert_eq!(s.handle(right), State::Cancelled);
    }

    #[test]
    fn dragging_up_and_left_selects_the_same_rect() {
        let mut s = one_monitor();
        s.handle(Input::Press {
            button: Button::Left,
            at: p(109, 69),
        });
        s.handle(Input::Move { at: p(10, 20) });
        assert_eq!(s.rect(), Some(Rect::new(10, 20, 110, 70)));
    }

    /// 左に 100% の FHD（負の座標）、中央にプライマリの 150% の 4K、右に 100px の隙間を空けて 100% の FHD。
    fn three_monitors() -> Vec<MonitorInfo> {
        vec![
            MonitorInfo {
                rect: Rect::new(0, 0, 3840, 2160),
                dpi: 144,
            },
            MonitorInfo {
                rect: Rect::new(-1920, 0, 0, 1080),
                dpi: 96,
            },
            MonitorInfo {
                rect: Rect::new(3940, 0, 5860, 1080),
                dpi: 96,
            },
        ]
    }

    #[test]
    fn a_rect_inside_one_monitor_is_one_part() {
        let parts = split(Rect::new(100, 100, 1380, 820), &three_monitors());
        assert_eq!(
            parts,
            vec![Part {
                monitor: 0,
                rect: Rect::new(100, 100, 1380, 820),
            }]
        );
    }

    #[test]
    fn a_rect_across_a_negative_monitor_and_a_monitor_with_another_dpi_splits_left_to_right() {
        let parts = split(Rect::new(-100, 500, 200, 700), &three_monitors());
        assert_eq!(
            parts,
            vec![
                Part {
                    monitor: 1,
                    rect: Rect::new(-100, 500, 0, 700),
                },
                Part {
                    monitor: 0,
                    rect: Rect::new(0, 500, 200, 700),
                },
            ]
        );
    }

    #[test]
    fn the_gap_between_monitors_belongs_to_no_part() {
        let parts = split(Rect::new(3800, 100, 4000, 300), &three_monitors());
        assert_eq!(
            parts,
            vec![
                Part {
                    monitor: 0,
                    rect: Rect::new(3800, 100, 3840, 300),
                },
                Part {
                    monitor: 2,
                    rect: Rect::new(3940, 100, 4000, 300),
                },
            ]
        );
    }

    #[test]
    fn the_area_below_a_shorter_monitor_belongs_to_no_part() {
        let parts = split(Rect::new(-50, 1000, 50, 1200), &three_monitors());
        assert_eq!(
            parts,
            vec![
                Part {
                    monitor: 1,
                    rect: Rect::new(-50, 1000, 0, 1080),
                },
                Part {
                    monitor: 0,
                    rect: Rect::new(0, 1000, 50, 1200),
                },
            ]
        );
    }

    #[test]
    fn the_size_label_shows_the_physical_size_of_the_range() {
        let mut s = Selection::new(three_monitors(), p(0, 0));
        assert_eq!(s.size_label(), None);
        s.handle(Input::Press {
            button: Button::Left,
            at: p(100, 100),
        });
        s.handle(Input::Move { at: p(1379, 819) });
        assert_eq!(s.size_label().as_deref(), Some("1280 × 720"));
    }

    #[test]
    fn the_size_label_joins_the_size_on_each_monitor_with_plus() {
        let mut s = Selection::new(three_monitors(), p(0, 0));
        s.handle(Input::Press {
            button: Button::Left,
            at: p(-100, 500),
        });
        s.handle(Input::Move { at: p(199, 699) });
        assert_eq!(s.size_label().as_deref(), Some("100 × 200 ＋ 200 × 200"));
    }

    fn arrow(direction: Direction, shift: bool) -> Input {
        Input::Arrow { direction, shift }
    }

    #[test]
    fn an_arrow_moves_the_cursor_one_pixel_and_shift_arrow_ten_pixels() {
        let mut s = Selection::new(
            vec![MonitorInfo {
                rect: Rect::new(0, 0, 1920, 1080),
                dpi: 96,
            }],
            p(500, 500),
        );
        s.handle(arrow(Direction::Right, false));
        assert_eq!(s.cursor(), p(501, 500));
        s.handle(arrow(Direction::Down, true));
        assert_eq!(s.cursor(), p(501, 510));
        s.handle(arrow(Direction::Left, true));
        assert_eq!(s.cursor(), p(491, 510));
        s.handle(arrow(Direction::Up, false));
        assert_eq!(s.cursor(), p(491, 509));
        assert_eq!(s.state(), State::Idle);
        assert_eq!(s.rect(), None);
    }

    #[test]
    fn arrows_move_the_cursor_onto_an_adjacent_monitor_but_not_off_the_screens() {
        let mut s = Selection::new(three_monitors(), p(-1, 500));
        // 隣り合うモニタへは移れる
        s.handle(arrow(Direction::Right, false));
        assert_eq!(s.cursor(), p(0, 500));
        // 隙間には入らず、モニタの端で止まる
        let mut s = Selection::new(three_monitors(), p(3835, 100));
        s.handle(arrow(Direction::Right, true));
        assert_eq!(s.cursor(), p(3839, 100));
        // 画面の外には出ない
        let mut s = Selection::new(three_monitors(), p(-1915, 5));
        s.handle(arrow(Direction::Left, true));
        s.handle(arrow(Direction::Up, true));
        assert_eq!(s.cursor(), p(-1920, 0));
        // 短いモニタの下（どのモニタでもない部分）にも入らない
        let mut s = Selection::new(three_monitors(), p(-10, 1075));
        s.handle(arrow(Direction::Down, true));
        assert_eq!(s.cursor(), p(-10, 1079));
    }

    #[test]
    fn space_sets_the_start_then_arrows_extend_and_space_again_confirms() {
        let mut s = Selection::new(three_monitors(), p(100, 200));
        assert_eq!(s.handle(Input::Space), State::Selecting);
        assert_eq!(s.rect(), Some(Rect::new(100, 200, 101, 201)));
        s.handle(arrow(Direction::Right, true));
        s.handle(arrow(Direction::Right, false));
        s.handle(arrow(Direction::Down, true));
        assert_eq!(s.state(), State::Selecting);
        assert_eq!(s.rect(), Some(Rect::new(100, 200, 112, 211)));
        assert_eq!(s.handle(Input::Space), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(100, 200, 112, 211)));
    }

    #[test]
    fn space_after_moving_the_mouse_starts_at_the_mouse() {
        let mut s = one_monitor();
        s.handle(Input::Move { at: p(300, 400) });
        s.handle(Input::Space);
        s.handle(Input::Move { at: p(250, 350) });
        assert_eq!(s.handle(Input::Space), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(250, 350, 301, 401)));
    }

    #[test]
    fn escape_after_space_starts_over() {
        let mut s = one_monitor();
        s.handle(Input::Space);
        assert_eq!(s.handle(Input::Escape), State::Idle);
        assert_eq!(s.rect(), None);
    }

    #[test]
    fn ctrl_a_confirms_the_whole_monitor_under_the_cursor() {
        let mut s = Selection::new(three_monitors(), p(-500, 300));
        assert_eq!(s.handle(Input::SelectMonitor), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(-1920, 0, 0, 1080)));
        assert_eq!(s.size_label().as_deref(), Some("1920 × 1080"));
    }

    #[test]
    fn ctrl_a_while_selecting_replaces_the_range_with_the_monitor() {
        let mut s = Selection::new(three_monitors(), p(0, 0));
        start_drag(&mut s);
        assert_eq!(s.handle(Input::SelectMonitor), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(0, 0, 3840, 2160)));
    }

    #[test]
    fn ctrl_shift_a_confirms_the_bounding_rect_of_all_monitors() {
        let mut s = Selection::new(three_monitors(), p(100, 100));
        assert_eq!(s.handle(Input::SelectAllMonitors), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(-1920, 0, 5860, 2160)));
        let parts = s.parts();
        assert_eq!(
            parts.iter().map(|part| part.monitor).collect::<Vec<_>>(),
            vec![1, 0, 2]
        );
    }

    fn precise() -> Selection {
        one_monitor().with_mode(Mode::Precise)
    }

    fn drag(s: &mut Selection, from: Point, to: Point) -> State {
        s.handle(Input::Press {
            button: Button::Left,
            at: from,
        });
        s.handle(Input::Move { at: to });
        s.handle(Input::Release {
            button: Button::Left,
            at: to,
        })
    }

    #[test]
    fn tab_toggles_between_normal_and_precise_mode() {
        let mut s = one_monitor();
        assert_eq!(s.mode(), Mode::Normal);
        s.handle(Input::ToggleMode);
        assert_eq!(s.mode(), Mode::Precise);
        s.handle(Input::ToggleMode);
        assert_eq!(s.mode(), Mode::Normal);
        assert_eq!(s.state(), State::Idle);
    }

    #[test]
    fn releasing_in_precise_mode_adjusts_instead_of_confirming() {
        let mut s = one_monitor();
        s.handle(Input::ToggleMode);
        let state = drag(&mut s, p(10, 20), p(109, 69));
        assert_eq!(state, State::Adjusting);
        assert_eq!(s.rect(), Some(Rect::new(10, 20, 110, 70)));
    }

    /// 精密モードで (100, 100)〜(199, 199) を選び、調整の状態にする。
    fn adjusting() -> Selection {
        let mut s = precise();
        assert_eq!(drag(&mut s, p(100, 100), p(199, 199)), State::Adjusting);
        assert_eq!(s.rect(), Some(Rect::new(100, 100, 200, 200)));
        s
    }

    #[test]
    fn the_default_mode_comes_from_the_settings_and_tab_only_changes_this_selection() {
        let mut s = precise();
        assert_eq!(s.mode(), Mode::Precise);
        s.handle(Input::ToggleMode);
        assert_eq!(s.mode(), Mode::Normal);
        assert_eq!(drag(&mut s, p(10, 20), p(109, 69)), State::Confirmed);
    }

    #[test]
    fn dragging_a_corner_handle_resizes_two_edges() {
        let mut s = adjusting();
        // 右下のつまみは、範囲のすぐ外の (200, 200) を中心にしている
        assert_eq!(drag(&mut s, p(201, 201), p(251, 231)), State::Adjusting);
        assert_eq!(s.rect(), Some(Rect::new(100, 100, 250, 230)));
        assert_eq!(drag(&mut s, p(99, 99), p(89, 109)), State::Adjusting);
        assert_eq!(s.rect(), Some(Rect::new(90, 110, 250, 230)));
    }

    #[test]
    fn dragging_an_edge_handle_resizes_only_that_edge() {
        let mut s = adjusting();
        // 上の辺のつまみ（中央）
        drag(&mut s, p(150, 100), p(170, 80));
        assert_eq!(s.rect(), Some(Rect::new(100, 80, 200, 200)));
        // 左の辺のつまみ
        drag(&mut s, p(100, 140), p(130, 0));
        assert_eq!(s.rect(), Some(Rect::new(130, 80, 200, 200)));
    }

    #[test]
    fn a_handle_cannot_make_the_range_smaller_than_one_pixel() {
        let mut s = adjusting();
        drag(&mut s, p(200, 200), p(0, 0));
        assert_eq!(s.rect(), Some(Rect::new(100, 100, 101, 101)));
    }

    #[test]
    fn handles_grow_with_the_monitor_dpi() {
        let mut s = Selection::new(
            vec![MonitorInfo {
                rect: Rect::new(0, 0, 3840, 2160),
                dpi: 192,
            }],
            p(0, 0),
        )
        .with_mode(Mode::Precise);
        drag(&mut s, p(100, 100), p(199, 199));
        assert_eq!(s.handles()[0], Rect::new(93, 93, 107, 107));
        // 96 DPI なら外れる位置でも、200% ならつかめる
        drag(&mut s, p(91, 91), p(81, 91));
        assert_eq!(s.rect(), Some(Rect::new(90, 100, 200, 200)));
    }

    #[test]
    fn dragging_inside_the_range_moves_it() {
        let mut s = adjusting();
        s.handle(Input::Press {
            button: Button::Left,
            at: p(150, 150),
        });
        s.handle(Input::Move { at: p(160, 140) });
        assert_eq!(s.rect(), Some(Rect::new(110, 90, 210, 190)));
        assert_eq!(
            s.handle(Input::Release {
                button: Button::Left,
                at: p(170, 140),
            }),
            State::Adjusting
        );
        assert_eq!(s.rect(), Some(Rect::new(120, 90, 220, 190)));
        // 離したあとの移動では動かない
        s.handle(Input::Move { at: p(300, 300) });
        assert_eq!(s.rect(), Some(Rect::new(120, 90, 220, 190)));
    }

    #[test]
    fn moving_the_range_stops_at_the_edge_of_the_screens() {
        let mut s = adjusting();
        drag(&mut s, p(150, 150), p(-500, 150));
        assert_eq!(s.rect(), Some(Rect::new(0, 100, 100, 200)));
        for _ in 0..30 {
            s.handle(arrow(Direction::Up, true));
        }
        assert_eq!(s.rect(), Some(Rect::new(0, 0, 100, 100)));
    }

    #[test]
    fn arrows_move_the_range_by_one_pixel_and_shift_by_ten() {
        let mut s = adjusting();
        let cursor = s.cursor();
        s.handle(arrow(Direction::Right, false));
        s.handle(arrow(Direction::Down, true));
        assert_eq!(s.rect(), Some(Rect::new(101, 110, 201, 210)));
        s.handle(arrow(Direction::Left, true));
        s.handle(arrow(Direction::Up, false));
        assert_eq!(s.rect(), Some(Rect::new(91, 109, 191, 209)));
        // カーソルは動かさない
        assert_eq!(s.cursor(), cursor);
        assert_eq!(s.state(), State::Adjusting);
    }

    fn resize_arrow(direction: Direction, shift: bool) -> Input {
        Input::ResizeArrow { direction, shift }
    }

    #[test]
    fn ctrl_arrows_move_the_bottom_right_edges_and_shift_by_ten() {
        let mut s = adjusting();
        s.handle(resize_arrow(Direction::Right, false));
        s.handle(resize_arrow(Direction::Down, true));
        assert_eq!(s.rect(), Some(Rect::new(100, 100, 201, 210)));
        s.handle(resize_arrow(Direction::Left, true));
        s.handle(resize_arrow(Direction::Up, false));
        assert_eq!(s.rect(), Some(Rect::new(100, 100, 191, 209)));
        // 1 画素より小さくはならない
        for _ in 0..20 {
            s.handle(resize_arrow(Direction::Left, true));
        }
        assert_eq!(s.rect(), Some(Rect::new(100, 100, 101, 209)));
    }

    #[test]
    fn ctrl_arrows_move_the_cursor_like_arrows_before_adjusting() {
        let mut s = Selection::new(
            vec![MonitorInfo {
                rect: Rect::new(0, 0, 1920, 1080),
                dpi: 96,
            }],
            p(500, 500),
        );
        s.handle(resize_arrow(Direction::Right, true));
        assert_eq!(s.cursor(), p(510, 500));
    }

    #[test]
    fn enter_confirms_the_adjusted_range() {
        let mut s = adjusting();
        s.handle(arrow(Direction::Right, false));
        assert_eq!(s.handle(Input::Enter), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(101, 100, 201, 200)));
    }

    #[test]
    fn enter_does_nothing_before_adjusting() {
        let mut s = precise();
        assert_eq!(s.handle(Input::Enter), State::Idle);
        start_drag(&mut s);
        assert_eq!(s.handle(Input::Enter), State::Selecting);
    }

    #[test]
    fn a_double_click_inside_the_range_confirms_it() {
        let mut s = adjusting();
        // 1 回目の押下と離すのは、動かさない移動のドラッグになる
        drag(&mut s, p(150, 150), p(150, 150));
        assert_eq!(
            s.handle(Input::DoubleClick { at: p(150, 150) }),
            State::Confirmed
        );
        assert_eq!(s.rect(), Some(Rect::new(100, 100, 200, 200)));
    }

    #[test]
    fn pressing_outside_the_range_starts_selecting_again_from_there() {
        let mut s = adjusting();
        s.handle(Input::Press {
            button: Button::Left,
            at: p(500, 500),
        });
        assert_eq!(s.state(), State::Selecting);
        s.handle(Input::Move { at: p(549, 529) });
        assert_eq!(s.rect(), Some(Rect::new(500, 500, 550, 530)));
        assert_eq!(
            s.handle(Input::Release {
                button: Button::Left,
                at: p(549, 529),
            }),
            State::Adjusting
        );
    }

    #[test]
    fn a_click_outside_the_range_leaves_nothing_selected() {
        let mut s = adjusting();
        assert_eq!(drag(&mut s, p(500, 500), p(500, 500)), State::Idle);
        assert_eq!(s.rect(), None);
    }

    #[test]
    fn escape_while_adjusting_starts_over_then_escape_again_cancels() {
        let mut s = adjusting();
        assert_eq!(s.handle(Input::Escape), State::Idle);
        assert_eq!(s.rect(), None);
        assert_eq!(s.handle(Input::Escape), State::Cancelled);
    }

    #[test]
    fn right_click_while_dragging_a_handle_starts_over() {
        let mut s = adjusting();
        s.handle(Input::Press {
            button: Button::Left,
            at: p(200, 200),
        });
        s.handle(Input::Move { at: p(300, 300) });
        let right = Input::Press {
            button: Button::Right,
            at: p(300, 300),
        };
        assert_eq!(s.handle(right), State::Idle);
        // 押したままの左ボタンを離しても、選び直しのまま
        assert_eq!(
            s.handle(Input::Release {
                button: Button::Left,
                at: p(300, 300),
            }),
            State::Idle
        );
        assert_eq!(s.rect(), None);
        assert_eq!(s.handle(right), State::Cancelled);
    }

    #[test]
    fn space_twice_in_precise_mode_adjusts_and_a_third_space_confirms() {
        let mut s = precise();
        s.handle(Input::Move { at: p(10, 10) });
        s.handle(Input::Space);
        s.handle(arrow(Direction::Right, true));
        assert_eq!(s.handle(Input::Space), State::Adjusting);
        assert_eq!(s.rect(), Some(Rect::new(10, 10, 21, 11)));
        assert_eq!(s.handle(Input::Space), State::Confirmed);
    }

    #[test]
    fn ctrl_a_in_precise_mode_adjusts_the_whole_monitor() {
        let mut s = Selection::new(three_monitors(), p(-500, 300)).with_mode(Mode::Precise);
        assert_eq!(s.handle(Input::SelectMonitor), State::Adjusting);
        assert_eq!(s.rect(), Some(Rect::new(-1920, 0, 0, 1080)));
    }

    #[test]
    fn handles_are_shown_only_while_adjusting() {
        let mut s = precise();
        start_drag(&mut s);
        assert_eq!(s.handles(), vec![]);
        let mut s = adjusting();
        assert_eq!(
            s.handles(),
            vec![
                Rect::new(97, 97, 104, 104),
                Rect::new(197, 97, 204, 104),
                Rect::new(197, 197, 204, 204),
                Rect::new(97, 197, 104, 204),
                Rect::new(147, 97, 154, 104),
                Rect::new(197, 147, 204, 154),
                Rect::new(147, 197, 154, 204),
                Rect::new(97, 147, 104, 154),
            ]
        );
        s.handle(Input::Enter);
        assert_eq!(s.handles(), vec![]);
    }

    #[test]
    fn the_cursor_shape_follows_what_is_under_the_cursor_while_adjusting() {
        let mut s = adjusting();
        let shape_at = |s: &mut Selection, at| {
            s.handle(Input::Move { at });
            s.cursor_shape()
        };
        assert_eq!(shape_at(&mut s, p(100, 100)), CursorShape::ResizeNwse);
        assert_eq!(shape_at(&mut s, p(200, 100)), CursorShape::ResizeNesw);
        assert_eq!(shape_at(&mut s, p(150, 200)), CursorShape::ResizeNs);
        assert_eq!(shape_at(&mut s, p(200, 150)), CursorShape::ResizeWe);
        assert_eq!(shape_at(&mut s, p(150, 150)), CursorShape::Move);
        assert_eq!(shape_at(&mut s, p(500, 500)), CursorShape::Cross);
        assert_eq!(precise().cursor_shape(), CursorShape::Cross);
    }

    #[test]
    fn the_crosshair_is_always_shown_in_precise_mode_and_follows_the_setting_in_normal_mode() {
        let mut s = one_monitor();
        s.handle(Input::Move { at: p(30, 40) });
        assert_eq!(s.view().crosshair, None);
        s.handle(Input::ToggleMode);
        assert_eq!(s.view().crosshair, Some(p(30, 40)));
        let mut s = one_monitor().with_crosshair(true);
        s.handle(Input::Move { at: p(30, 40) });
        assert_eq!(s.view().crosshair, Some(p(30, 40)));
    }

    #[test]
    fn the_cursor_label_shows_physical_coordinates_and_the_size_while_dragging() {
        let mut s = one_monitor();
        s.handle(Input::Move { at: p(30, 40) });
        assert_eq!(s.cursor_label(), None);
        let mut s = precise();
        s.handle(Input::Move { at: p(30, 40) });
        assert_eq!(s.cursor_label().as_deref(), Some("30, 40"));
        s.handle(Input::Press {
            button: Button::Left,
            at: p(30, 40),
        });
        s.handle(Input::Move { at: p(129, 89) });
        assert_eq!(s.cursor_label().as_deref(), Some("129, 89   100 × 50"));
        s.handle(Input::Release {
            button: Button::Left,
            at: p(129, 89),
        });
        assert_eq!(s.cursor_label().as_deref(), Some("129, 89"));
        // つまみを引いている間もサイズを出す
        s.handle(Input::Press {
            button: Button::Left,
            at: p(130, 90),
        });
        s.handle(Input::Move { at: p(140, 90) });
        assert_eq!(s.cursor_label().as_deref(), Some("140, 90   110 × 50"));
    }

    #[test]
    fn the_magnifier_follows_the_cursor_only_in_precise_mode() {
        let mut s = one_monitor().with_crosshair(true);
        s.handle(Input::Move { at: p(30, 40) });
        assert_eq!(s.view().magnifier, None);
        s.handle(Input::ToggleMode);
        assert_eq!(s.view().magnifier, Some(p(30, 40)));
        s.handle(Input::Arrow {
            direction: Direction::Right,
            shift: false,
        });
        assert_eq!(s.view().magnifier, Some(p(31, 40)));
    }

    #[test]
    fn inputs_after_cancelling_are_ignored() {
        let mut s = one_monitor();
        s.handle(Input::Escape);
        start_drag(&mut s);
        assert_eq!(s.state(), State::Cancelled);
        assert_eq!(s.rect(), None);
    }
}
