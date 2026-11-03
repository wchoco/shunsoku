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
//! - ハイライトのないまま動かさずにクリックしたら（モードによらず）、`Esc` と同じく中止する。
//!   定型領域の登録では中止せず、何も選んでいない状態に戻す。
//! - 定型領域の登録（`registering`）は常に精密モードで、`Tab` は効かない。範囲は 1 枚のモニタの中に限り、
//!   ウィンドウを選んだら、その矩形をモニタの中に切り詰めて調整に入る。

use crate::adjust::{self, Grab};
pub use crate::adjust::{CursorShape, Direction, Handle};
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

/// 範囲選択の画面のモード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// ドラッグを離した時点で範囲が決まる。
    #[default]
    Normal,
    /// 範囲を調整してから確定する。十字線とカーソルの横の座標を常に出す。
    Precise,
}

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
    /// 定型領域の登録（`registering`）。
    registering: bool,
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

/// 範囲選択の画面に描くもの。Win32 層は入力の前後でこれを比べ、変わったときだけ描き直す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// 範囲。何も選んでいなければ None。
    pub rect: Option<Rect>,
    /// 範囲の左上の外側に出すサイズ（`Selection::size_label`）。
    pub size_label: Option<Vec<String>>,
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
            registering: false,
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
    /// 定型領域の登録では、その矩形を重なる面積が最も大きいモニタの中に切り詰めて調整に入る。
    fn choose_window(&mut self, highlight: Option<Highlight>) {
        if self.registering {
            let trimmed = highlight.and_then(|h| {
                split(h.rect, &self.monitors)
                    .into_iter()
                    .max_by_key(|part| u64::from(part.rect.width()) * u64::from(part.rect.height()))
                    .map(|part| part.rect)
            });
            if let Some(rect) = trimmed {
                self.settle(rect);
            }
            return;
        }
        if let Some(highlight) = highlight {
            self.windows.chosen = Some(highlight);
            self.anchor = None;
            self.state = State::Confirmed;
        }
    }

    /// 開いたときのモード（設定の既定のモード）。
    /// 定型領域の登録では設定によらず精密モード。
    pub fn with_mode(mut self, mode: Mode) -> Selection {
        if !self.registering {
            self.mode = mode;
        }
        self
    }

    /// 定型領域の登録の画面にする。常に精密モードで、`Tab` は効かない。範囲は 1 枚のモニタの中に限り、
    /// ウィンドウを選んだら、その矩形（モニタをまたげば重なる面積が最も大きいモニタの中に切り詰めたもの）で
    /// 調整に入る。確定すると `State::Confirmed` になり、`rect` が登録する範囲（スクリーン座標）。
    pub fn registering(mut self) -> Selection {
        self.registering = true;
        self.mode = Mode::Precise;
        self
    }

    /// 定型領域の登録の画面か（`registering`）。
    pub fn is_registering(&self) -> bool {
        self.registering
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
                        // していなければ取りやめる（定型領域の登録では選び直す）
                        let highlight = self.highlight();
                        self.reset();
                        if highlight.is_some() {
                            self.choose_window(highlight);
                        } else if !self.registering {
                            self.state = State::Cancelled;
                        }
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
            // 定型領域は 1 枚のモニタの中に限る
            Input::SelectAllMonitors if self.registering => {}
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
        if self.registering {
            return;
        }
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
                    self.adjusted = Some(grab.dragged(at, self.bounds()));
                }
            }
            Input::Release {
                button: Button::Left,
                at,
            } => {
                self.cursor = at;
                if let Some(grab) = self.grab.take() {
                    self.adjusted = Some(grab.dragged(at, self.bounds()));
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
                self.adjusted = Some(adjust::moved(rect, direction, shift, self.bounds()));
            }
            Input::ResizeArrow { direction, shift } => {
                self.adjusted = Some(adjust::resized(rect, direction, shift, self.bounds()));
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
        adjust::hit(rect, at, |len, at| self.scaled(len, at))
    }

    /// 96 DPI での長さ `len` を、`at` のあるモニタ（なければ 96 DPI）の物理ピクセルにする。
    fn scaled(&self, len: i32, at: Point) -> i32 {
        let dpi = self.monitor_at(at).map_or(96, |m| m.dpi) as i32;
        (len * dpi + 48) / 96
    }

    /// 範囲を動かせる・広げられる矩形。定型領域の登録では範囲のあるモニタ、ほかは全モニタを囲む矩形。
    fn bounds(&self) -> Rect {
        if let Some(home) = self.home() {
            return home;
        }
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

    /// 定型領域の登録で、範囲を収めるモニタ。始点（調整の状態なら範囲の左上）のあるモニタ。
    /// 登録でなければ、何も選んでいなければ None。
    fn home(&self) -> Option<Rect> {
        if !self.registering {
            return None;
        }
        let at = self.adjusted.map(|r| r.origin()).or(self.anchor)?;
        self.monitor_at(at).map(|m| m.rect)
    }

    /// 矢印でカーソルを動かす。
    fn move_cursor(&mut self, direction: Direction, shift: bool) {
        let (dx, dy) = adjust::step(direction, shift);
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
        let rect = Rect::new(
            a.x.min(c.x),
            a.y.min(c.y),
            a.x.max(c.x) + 1,
            a.y.max(c.y) + 1,
        );
        // 定型領域の登録では、始点のあるモニタの外にははみ出さない
        match self.home() {
            Some(home) => rect.intersect(&home).or(Some(rect)),
            None => Some(rect),
        }
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
            (State::Adjusting, Some(rect)) => {
                adjust::handle_rects(rect, |len, at| self.scaled(len, at))
            }
            _ => Vec::new(),
        }
    }

    /// カーソルの形。調整の状態で、つまみの上なら大きさを変える形、範囲の中なら動かす形。
    /// つかんでいる間は、つかんだものの形のまま。
    pub fn cursor_shape(&self) -> CursorShape {
        let (State::Adjusting, Some(rect)) = (self.state, self.adjusted) else {
            return CursorShape::Cross;
        };
        match self.grab {
            Some(grab) => grab.cursor_shape(),
            None => adjust::cursor_shape(self.hit(rect, self.cursor)),
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

    /// 範囲の左上の外側に出すサイズ（`1280 × 720`）。モニタをまたいでいれば、モニタごとのサイズを左から
    /// 並べる（画面では文言の `selection.size_plus` でつなぐ）。何も選んでいなければ None。
    pub fn size_label(&self) -> Option<Vec<String>> {
        let rect = self.rect()?;
        let parts = split(rect, &self.monitors);
        let label = if parts.len() <= 1 {
            vec![size_text(rect)]
        } else {
            parts.iter().map(|part| size_text(part.rect)).collect()
        };
        Some(label)
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
    fn a_click_without_dragging_cancels() {
        let mut s = one_monitor();
        s.handle(Input::Press {
            button: Button::Left,
            at: p(10, 20),
        });
        let state = s.handle(Input::Release {
            button: Button::Left,
            at: p(10, 20),
        });
        assert_eq!(state, State::Cancelled);
        assert_eq!(s.chosen_window(), None);
    }

    #[test]
    fn space_twice_on_the_same_pixel_confirms_a_one_pixel_range() {
        let mut s = one_monitor();
        s.handle(Input::Move { at: p(10, 20) });
        s.handle(Input::Space);
        assert_eq!(s.handle(Input::Space), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(10, 20, 11, 21)));
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
        assert_eq!(s.size_label(), Some(vec!["1280 × 720".to_owned()]));
    }

    #[test]
    fn the_size_label_has_the_size_on_each_monitor_from_the_left() {
        let mut s = Selection::new(three_monitors(), p(0, 0));
        s.handle(Input::Press {
            button: Button::Left,
            at: p(-100, 500),
        });
        s.handle(Input::Move { at: p(199, 699) });
        assert_eq!(
            s.size_label(),
            Some(vec!["100 × 200".to_owned(), "200 × 200".to_owned()])
        );
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
        assert_eq!(s.size_label(), Some(vec!["1920 × 1080".to_owned()]));
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
    fn a_click_without_dragging_in_precise_mode_cancels() {
        let mut s = precise();
        assert_eq!(drag(&mut s, p(500, 500), p(500, 500)), State::Cancelled);
    }

    #[test]
    fn a_click_outside_the_adjusted_range_cancels() {
        let mut s = adjusting();
        assert_eq!(drag(&mut s, p(500, 500), p(500, 500)), State::Cancelled);
    }

    #[test]
    fn a_one_pixel_range_in_precise_mode_is_confirmed_with_enter() {
        let mut s = precise();
        s.handle(Input::Move { at: p(10, 20) });
        s.handle(Input::Space);
        assert_eq!(s.handle(Input::Space), State::Adjusting);
        assert_eq!(s.handle(Input::Enter), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(10, 20, 11, 21)));
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

    /// 定型領域の登録: 1920×1080 の 2 枚を横に並べる（左 96 DPI、右 144 DPI）。
    fn registering() -> Selection {
        Selection::new(
            vec![
                MonitorInfo {
                    rect: Rect::new(0, 0, 1920, 1080),
                    dpi: 96,
                },
                MonitorInfo {
                    rect: Rect::new(1920, 0, 3840, 1080),
                    dpi: 144,
                },
            ],
            p(0, 0),
        )
        .registering()
    }

    #[test]
    fn a_click_without_dragging_while_registering_starts_over_instead_of_cancelling() {
        let mut s = registering();
        assert_eq!(drag(&mut s, p(500, 500), p(500, 500)), State::Idle);
        assert_eq!(s.rect(), None);
    }

    #[test]
    fn registering_always_uses_precise_mode_and_tab_does_nothing() {
        let mut s = registering();
        assert_eq!(s.mode(), Mode::Precise);
        s.handle(Input::ToggleMode);
        assert_eq!(s.mode(), Mode::Precise);
        // 調整の状態でも効かない
        s.handle(Input::SelectMonitor);
        assert_eq!(s.state(), State::Adjusting);
        s.handle(Input::ToggleMode);
        assert_eq!(s.mode(), Mode::Precise);
        // 通常モードを既定にしていても精密モードで開く
        let s = Selection::new(Vec::new(), p(0, 0))
            .with_mode(Mode::Normal)
            .registering();
        assert_eq!(s.mode(), Mode::Precise);
    }

    #[test]
    fn registering_keeps_the_range_inside_the_monitor_where_it_started() {
        let mut s = registering();
        // 左のモニタから右のモニタへ引いても、左のモニタの中で止まる
        s.handle(Input::Press {
            button: Button::Left,
            at: p(1800, 100),
        });
        s.handle(Input::Move { at: p(2100, 199) });
        assert_eq!(s.rect(), Some(Rect::new(1800, 100, 1920, 200)));
        s.handle(Input::Release {
            button: Button::Left,
            at: p(2100, 199),
        });
        assert_eq!(s.state(), State::Adjusting);
        assert_eq!(s.rect(), Some(Rect::new(1800, 100, 1920, 200)));
        // 調整でも、動かす・広げるのはそのモニタの中まで
        s.handle(Input::Arrow {
            direction: Direction::Right,
            shift: true,
        });
        assert_eq!(s.rect(), Some(Rect::new(1800, 100, 1920, 200)));
        s.handle(Input::Press {
            button: Button::Left,
            at: p(1850, 150),
        });
        s.handle(Input::Move { at: p(2850, 150) });
        assert_eq!(s.rect(), Some(Rect::new(1800, 100, 1920, 200)));
        s.handle(Input::Release {
            button: Button::Left,
            at: p(2850, 150),
        });
        s.handle(Input::Press {
            button: Button::Left,
            at: p(1800, 100),
        });
        s.handle(Input::Move { at: p(1500, -50) });
        assert_eq!(s.rect(), Some(Rect::new(1500, 0, 1920, 200)));
        s.handle(Input::Release {
            button: Button::Left,
            at: p(1500, -50),
        });
        // 全モニタを囲む範囲は選べない
        s.handle(Input::Escape);
        s.handle(Input::Move { at: p(2000, 10) });
        s.handle(Input::SelectAllMonitors);
        assert_eq!(s.state(), State::Idle);
        s.handle(Input::SelectMonitor);
        assert_eq!(s.rect(), Some(Rect::new(1920, 0, 3840, 1080)));
        s.handle(Input::Enter);
        assert_eq!(s.state(), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(1920, 0, 3840, 1080)));
    }

    fn window(id: isize, rect: Rect) -> WindowInfo {
        WindowInfo {
            id: WindowId(id),
            rect,
            name: format!("window {id}"),
            parent: None,
            kind: WindowKind::Normal,
        }
    }

    #[test]
    fn registering_a_window_trims_it_to_the_monitor_it_overlaps_most_and_starts_adjusting() {
        let mut s = registering();
        s.set_windows(vec![
            // 右のモニタに多く重なる（左 100 列、右 300 列）。上と下は画面の外にはみ出す
            window(1, Rect::new(1820, -20, 2220, 1100)),
            window(2, Rect::new(100, 100, 500, 400)),
        ]);
        s.handle(Input::Move { at: p(1850, 500) });
        s.handle(Input::Modifiers {
            shift: true,
            ctrl: false,
        });
        s.handle(Input::Press {
            button: Button::Left,
            at: p(1850, 500),
        });
        let state = s.handle(Input::Release {
            button: Button::Left,
            at: p(1850, 500),
        });
        assert_eq!(state, State::Adjusting);
        assert_eq!(s.rect(), Some(Rect::new(1920, 0, 2220, 1080)));
        assert_eq!(s.chosen_window(), None);
        // 調整してから Enter で登録する範囲が決まる
        s.handle(Input::Modifiers {
            shift: false,
            ctrl: false,
        });
        s.handle(Input::ResizeArrow {
            direction: Direction::Left,
            shift: true,
        });
        assert_eq!(s.handle(Input::Enter), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(1920, 0, 2210, 1080)));
        assert_eq!(s.chosen_window(), None);

        // Shift+Enter でも同じ
        let mut s = registering();
        s.set_windows(vec![window(2, Rect::new(100, 100, 500, 400))]);
        s.handle(Input::Move { at: p(200, 200) });
        s.handle(Input::Modifiers {
            shift: true,
            ctrl: false,
        });
        assert_eq!(s.handle(Input::Enter), State::Adjusting);
        assert_eq!(s.rect(), Some(Rect::new(100, 100, 500, 400)));
    }
}
