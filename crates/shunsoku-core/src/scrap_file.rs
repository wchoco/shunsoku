//! スクラップファイル（`.shunsoku`）の書き出しと読み込み（ADR-0009）。
//!
//! スクラップを、書き込みを直せる形のまま保存して開き直すためのファイル。中身は元の画像（トリム前）、
//! 書き込み、トリムの範囲と、形式の版。位置・倍率・見た目の個別値・元に戻す / やり直すの履歴は入れない
//! （開いたスクラップは既定の状態で浮かべる）。
//!
//! 並びは次のとおり（数はリトルエンディアン）。
//!
//! ```text
//! MAGIC（13 バイト） | 版（u32） | 記録の長さ（u32） | 記録（UTF-8 の TOML） | 元の画像の PNG（残り全部。白紙スクラップはなし）
//! ```
//!
//! 記録の `[source]` の `kind` が中身の種類で、画像（`"image"`、大きさを持ち、後ろに PNG が続く）と白紙
//! スクラップ（`"blank"`、大きさと `background`（`#RRGGBB` か `transparent`）を持ち、後ろに何も続かない）。
//! 種類と後ろの PNG の有無が食い違うファイルは形式の誤りとして拒む。
//!
//! ファイルは外から来るデータで、読み込みは管理者として動く常駐本体の中で走る。大きな確保をしないよう、
//! ファイル・記録・画像の大きさと書き込みの数に上限を置き、PNG は画素を読む前に大きさを記録と突き合わせる。
//! 記録は知らないキーも誤りとして拒む。

use std::io::Read;
use std::path::Path;

use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value};

use crate::annotate::{Annotation, Annotations, Font, Shape, Style};
use crate::geometry::{Point, Rect, Size};
use crate::image::Image;
use crate::scrap::{Background, ScrapModel};
use crate::settings::{ANNOTATION_FONT_SIZE_RANGE, ANNOTATION_WIDTH_RANGE, Rgb};

/// スクラップファイルの拡張子。
pub const EXTENSION: &str = "shunsoku";

/// 先頭の識別子。PNG と同じく、上位ビットの立ったバイトと改行で、テキストとしての変換による壊れを見分ける。
pub const MAGIC: &[u8; 13] = b"\x89SHUNSOKU\r\n\x1a\n";

/// いまの形式の版。これより新しい版のファイルは開かない（`LoadError::NewerVersion`）。
pub const VERSION: u32 = 1;

/// ファイルの大きさの上限（バイト）。画像の上限の PNG が収まる大きさ。
pub const MAX_FILE_BYTES: u64 = 320 * 1024 * 1024;
/// 記録（TOML）の大きさの上限（バイト）。
pub const MAX_RECORD_BYTES: usize = 8 * 1024 * 1024;
/// 画像の一辺の上限（画素）。
pub const MAX_SIDE: u32 = 32_768;
/// 画像の画素の数の上限（BGRA で 256 MiB）。
pub const MAX_PIXELS: u64 = 1 << 26;
/// 書き込みの数の上限。
pub const MAX_ANNOTATIONS: usize = 100_000;
/// 書き込みの座標の絶対値の上限（元の画像の画素）。画像の外へ動かした書き込みも入る広さ。
pub const MAX_COORDINATE: i64 = 1 << 24;
/// 文字の書き込みの文字の数の上限。
pub const MAX_TEXT_CHARS: usize = 65_536;
/// 書体の名前の長さの上限（バイト）。
pub const MAX_FONT_FAMILY_BYTES: usize = 256;

/// 版と記録の長さの分のバイト数。
const HEADER_NUMBERS: usize = 8;

/// スクラップファイルの中身。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrapFile {
    pub content: Content,
    /// トリムしていれば、表示する範囲（元の画像の画素）。
    pub trim: Option<Rect>,
    /// 書き込み（描いた順。元の画像の画素の座標）。
    pub annotations: Vec<Annotation>,
}

/// スクラップの下地。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// 元の画像（トリム前）。
    Image(Image),
    /// 白紙スクラップの下地（大きさと背景色。画素は持たず、開くときに背景色で塗る）。
    Blank { size: Size, background: Background },
}

impl Content {
    /// 下地の大きさ（画素）。
    pub fn size(&self) -> Size {
        match self {
            Content::Image(image) => image.size(),
            Content::Blank { size, .. } => *size,
        }
    }
}

impl ScrapFile {
    /// スクラップ `model` と元の画像 `image` から作る（保存するとき）。白紙スクラップなら、塗った下地の
    /// `image` ではなく背景色と大きさを持つ（開くときに塗り直す）。
    pub fn of(model: &ScrapModel, image: &Image) -> ScrapFile {
        let content = match model.background() {
            Some(background) => Content::Blank {
                size: model.image_size,
                background,
            },
            None => Content::Image(image.clone()),
        };
        ScrapFile {
            content,
            trim: model.trim_rect(),
            annotations: model.annotations().items().to_vec(),
        }
    }

    /// 開いたスクラップの状態。表示する画像（トリムしていればその範囲）の中心を `monitor`（スクリーン座標）の
    /// 中心に置き、倍率は 100%、見た目の個別値はなく、履歴は空。白紙スクラップは背景色を持つ（下地は
    /// `ScrapModel::blank_image` で塗る）。
    pub fn into_scrap(self, monitor: Rect) -> (ScrapModel, Content) {
        let size = self.content.size();
        let shown = self.trim.map_or(size, |r| r.size());
        let center = |from: i32, length: u32, shown: u32| {
            let at = i64::from(from) + (i64::from(length) - i64::from(shown)) / 2;
            at.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
        };
        let position = Point::new(
            center(monitor.left, monitor.width(), shown.width),
            center(monitor.top, monitor.height(), shown.height),
        );
        let background = match self.content {
            Content::Image(_) => None,
            Content::Blank { background, .. } => Some(background),
        };
        let model = ScrapModel::restored(
            position,
            size,
            background,
            self.trim,
            Annotations::from_items(self.annotations),
        );
        (model, self.content)
    }
}

/// 書き出せなかった。
#[derive(Debug)]
pub enum EncodeError {
    Png(png::EncodingError),
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::Png(e) => write!(f, "could not encode the image: {e}"),
        }
    }
}

impl std::error::Error for EncodeError {}

/// 読めなかったわけ。それぞれ通知で別の理由を出す。
#[derive(Debug)]
pub enum LoadError {
    /// ファイルを読めなかった（ない、権限がない など）。
    Unreadable(std::io::Error),
    /// 先頭の識別子が違う（スクラップファイルでない）。
    NotScrapFile,
    /// 知らない新しい版。
    NewerVersion(u32),
    /// 記録が壊れている（途中で切れている、TOML として読めない、項目が足りない・知らない・型が違う）。
    BadRecord(String),
    /// 元の画像の PNG が壊れている（読めない、形式が違う）。
    BadImage(String),
    /// PNG の大きさが記録の大きさと違う。
    SizeMismatch { recorded: Size, image: Size },
    /// ファイル・記録・画像の大きさ、または書き込みの数が上限を超える。
    TooLarge,
    /// トリムの範囲が画像の外、書き込みの座標・太さ・文字の大きさが範囲の外。
    OutOfRange(String),
}

/// 読めなかったわけの大きな分け方（CLI の応答の種類）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadErrorKind {
    /// ファイルを読めない。
    Unreadable,
    /// スクラップファイルとして正しくない。
    Malformed,
    /// 新しい版の shunsoku で作ったもの。
    NewerVersion,
}

impl LoadError {
    pub fn kind(&self) -> LoadErrorKind {
        match self {
            LoadError::Unreadable(_) => LoadErrorKind::Unreadable,
            LoadError::NewerVersion(_) => LoadErrorKind::NewerVersion,
            LoadError::NotScrapFile
            | LoadError::BadRecord(_)
            | LoadError::BadImage(_)
            | LoadError::SizeMismatch { .. }
            | LoadError::TooLarge
            | LoadError::OutOfRange(_) => LoadErrorKind::Malformed,
        }
    }

    /// 通知に出す文言のキー（`locales/*.toml` の `scrap_file.*`）。
    pub fn message_key(&self) -> &'static str {
        match self {
            LoadError::Unreadable(_) => "scrap_file.unreadable",
            LoadError::NotScrapFile => "scrap_file.not_scrap_file",
            LoadError::NewerVersion(_) => "scrap_file.newer_version",
            LoadError::BadRecord(_) => "scrap_file.bad_record",
            LoadError::BadImage(_) => "scrap_file.bad_image",
            LoadError::SizeMismatch { .. } => "scrap_file.size_mismatch",
            LoadError::TooLarge => "scrap_file.too_large",
            LoadError::OutOfRange(_) => "scrap_file.out_of_range",
        }
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Unreadable(e) => write!(f, "could not read the file: {e}"),
            LoadError::NotScrapFile => f.write_str("not a scrap file"),
            LoadError::NewerVersion(v) => write!(f, "made by a newer version (format {v})"),
            LoadError::BadRecord(why) => write!(f, "broken record: {why}"),
            LoadError::BadImage(why) => write!(f, "broken image: {why}"),
            LoadError::SizeMismatch { recorded, image } => write!(
                f,
                "the image is {}x{} but the record says {}x{}",
                image.width, image.height, recorded.width, recorded.height
            ),
            LoadError::TooLarge => f.write_str("too large"),
            LoadError::OutOfRange(why) => write!(f, "out of range: {why}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// バイト列に書き出す。
pub fn encode(file: &ScrapFile) -> Result<Vec<u8>, EncodeError> {
    let mut doc = DocumentMut::new();
    let mut source = Table::new();
    let size = file.content.size();
    match &file.content {
        Content::Image(_) => {
            source.insert("kind", toml_edit::value("image"));
        }
        Content::Blank { .. } => {
            source.insert("kind", toml_edit::value("blank"));
        }
    }
    source.insert("width", toml_edit::value(i64::from(size.width)));
    source.insert("height", toml_edit::value(i64::from(size.height)));
    if let Content::Blank { background, .. } = &file.content {
        source.insert("background", toml_edit::value(background.to_string()));
    }
    doc.insert("source", Item::Table(source));
    if let Some(trim) = file.trim {
        let mut table = Table::new();
        table.insert("x", toml_edit::value(i64::from(trim.left)));
        table.insert("y", toml_edit::value(i64::from(trim.top)));
        table.insert("width", toml_edit::value(i64::from(trim.width())));
        table.insert("height", toml_edit::value(i64::from(trim.height())));
        doc.insert("trim", Item::Table(table));
    }
    if !file.annotations.is_empty() {
        let mut tables = ArrayOfTables::new();
        for annotation in &file.annotations {
            tables.push(annotation_table(annotation));
        }
        doc.insert("annotations", Item::ArrayOfTables(tables));
    }
    let record = doc.to_string();

    let mut out = Vec::with_capacity(MAGIC.len() + HEADER_NUMBERS + record.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    let length = u32::try_from(record.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(record.as_bytes());
    match &file.content {
        Content::Image(image) => image.write_png(&mut out).map_err(EncodeError::Png)?,
        Content::Blank { .. } => {}
    }
    Ok(out)
}

fn point_value(p: Point) -> Value {
    let mut array = Array::new();
    array.push(i64::from(p.x));
    array.push(i64::from(p.y));
    Value::Array(array)
}

fn annotation_table(annotation: &Annotation) -> Table {
    let mut table = Table::new();
    let mut ends = |kind: &str, from: Point, to: Point| {
        table.insert("shape", toml_edit::value(kind));
        table.insert("from", toml_edit::value(point_value(from)));
        table.insert("to", toml_edit::value(point_value(to)));
    };
    match &annotation.shape {
        Shape::Rect { from, to } => ends("rect", *from, *to),
        Shape::Ellipse { from, to } => ends("ellipse", *from, *to),
        Shape::Arrow { from, to } => ends("arrow", *from, *to),
        Shape::Line { from, to } => ends("line", *from, *to),
        Shape::Freehand { points } => {
            table.insert("shape", toml_edit::value("freehand"));
            let array: Array = points.iter().map(|&p| point_value(p)).collect();
            table.insert("points", toml_edit::value(array));
        }
        Shape::Text { at, text, font } => {
            table.insert("shape", toml_edit::value("text"));
            table.insert("at", toml_edit::value(point_value(*at)));
            table.insert("text", toml_edit::value(text.as_str()));
            table.insert("font", toml_edit::value(font.family.as_str()));
            table.insert("font_size", toml_edit::value(i64::from(font.size)));
        }
    }
    table.insert(
        "color",
        toml_edit::value(annotation.style.color.to_string()),
    );
    table.insert("width", toml_edit::value(i64::from(annotation.style.width)));
    table
}

/// `path` のファイルを読む。上限より大きいファイルは、上限の分だけ読んで止める。
///
/// パスは CLI（`open`）から届くこともあり、常駐本体は管理者として動くことがあるので、ふつうのファイルでなければ
/// （デバイス、名前付きパイプ、フォルダ など）読まずに `Unreadable` にする。Windows では、開いた先が名前付きパイプ
/// だったときに相手にこのプロセスとして振る舞わせないよう、なりすましの範囲を識別だけにして開く。
pub fn read(path: &Path) -> Result<ScrapFile, LoadError> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        /// `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`。
        const IDENTIFICATION_ONLY: u32 = 0x0010_0000 | 0x0001_0000;
        options.security_qos_flags(IDENTIFICATION_ONLY);
    }
    let file = options.open(path).map_err(LoadError::Unreadable)?;
    let metadata = file.metadata().map_err(LoadError::Unreadable)?;
    if !metadata.is_file() {
        return Err(LoadError::Unreadable(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a plain file",
        )));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(LoadError::Unreadable)?;
    decode(&bytes)
}

/// バイト列から読む。
pub fn decode(bytes: &[u8]) -> Result<ScrapFile, LoadError> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(LoadError::TooLarge);
    }
    let rest = bytes
        .strip_prefix(MAGIC.as_slice())
        .ok_or(LoadError::NotScrapFile)?;
    let (numbers, rest) = rest
        .split_at_checked(HEADER_NUMBERS)
        .ok_or_else(|| bad("the header is cut short"))?;
    let version = u32::from_le_bytes(numbers[..4].try_into().unwrap_or_default());
    let length = u32::from_le_bytes(numbers[4..].try_into().unwrap_or_default());
    if version == 0 {
        return Err(bad("format version 0"));
    }
    if version > VERSION {
        return Err(LoadError::NewerVersion(version));
    }
    let length = usize::try_from(length).map_err(|_| LoadError::TooLarge)?;
    if length > MAX_RECORD_BYTES {
        return Err(LoadError::TooLarge);
    }
    let (record, payload) = rest
        .split_at_checked(length)
        .ok_or_else(|| bad("the record is cut short"))?;
    let record = std::str::from_utf8(record).map_err(|_| bad("the record is not UTF-8"))?;
    let doc: DocumentMut = record
        .parse()
        .map_err(|e: toml_edit::TomlError| bad(e.message()))?;
    let root = doc.as_table();
    only_keys(root, "", &["source", "trim", "annotations"])?;

    let source = root
        .get("source")
        .and_then(Item::as_table)
        .ok_or_else(|| bad("no [source]"))?;
    let kind = source
        .get("kind")
        .and_then(Item::as_str)
        .ok_or_else(|| bad("no source.kind"))?;
    let content = match kind {
        "image" => {
            only_keys(source, "source", &["kind", "width", "height"])?;
            Content::Image(decode_png(payload, source_size(source)?)?)
        }
        "blank" => {
            only_keys(source, "source", &["kind", "width", "height", "background"])?;
            let size = source_size(source)?;
            // 開くときに背景色で塗るので、画像と同じ画素の数の上限を置く
            if u64::from(size.width) * u64::from(size.height) > MAX_PIXELS {
                return Err(LoadError::TooLarge);
            }
            let background = string(source, "source", "background")?
                .parse()
                .map_err(|_| bad("source.background is not #RRGGBB or transparent"))?;
            if !payload.is_empty() {
                return Err(bad("a blank scrap has an image after the record"));
            }
            Content::Blank { size, background }
        }
        other => return Err(bad(format!("unknown source.kind {other:?}"))),
    };
    let size = content.size();

    let trim = match root.get("trim") {
        None => None,
        Some(item) => {
            let table = item.as_table().ok_or_else(|| bad("trim is not a table"))?;
            Some(trim_rect(table, size)?)
        }
    };

    let annotations = match root.get("annotations") {
        None => Vec::new(),
        Some(item) => {
            let tables = item
                .as_array_of_tables()
                .ok_or_else(|| bad("annotations is not an array of tables"))?;
            if tables.len() > MAX_ANNOTATIONS {
                return Err(LoadError::TooLarge);
            }
            tables
                .iter()
                .enumerate()
                .map(|(i, table)| read_annotation(table, i))
                .collect::<Result<Vec<_>, _>>()?
        }
    };

    Ok(ScrapFile {
        content,
        trim,
        annotations,
    })
}

fn bad(why: impl Into<String>) -> LoadError {
    LoadError::BadRecord(why.into())
}

/// `table` のキーが `allowed` のどれかだけか。
fn only_keys(table: &Table, path: &str, allowed: &[&str]) -> Result<(), LoadError> {
    match table.iter().find(|(key, _)| !allowed.contains(key)) {
        Some((key, _)) if path.is_empty() => Err(bad(format!("unknown key {key:?}"))),
        Some((key, _)) => Err(bad(format!("unknown key {path}.{key}"))),
        None => Ok(()),
    }
}

/// `table` の整数 `key`。なければ・整数でなければ記録の誤り。
fn integer(table: &Table, path: &str, key: &str) -> Result<i64, LoadError> {
    table
        .get(key)
        .and_then(Item::as_integer)
        .ok_or_else(|| bad(format!("{path}.{key} is missing or not an integer")))
}

/// `[source]` の大きさ。
fn source_size(source: &Table) -> Result<Size, LoadError> {
    Ok(Size::new(
        side(source, "source", "width")?,
        side(source, "source", "height")?,
    ))
}

/// 画像の一辺。0 は記録の誤り、上限を超えたら `TooLarge`。
fn side(table: &Table, path: &str, key: &str) -> Result<u32, LoadError> {
    let n = integer(table, path, key)?;
    if n < 1 {
        return Err(bad(format!("{path}.{key} must be at least 1")));
    }
    if n > i64::from(MAX_SIDE) {
        return Err(LoadError::TooLarge);
    }
    Ok(n as u32)
}

/// 元の画像の PNG を読む。大きさは画素を読む前に記録の `size` と突き合わせ、8 bit の RGBA だけを受ける
/// （書き出すのはその形だけ）。
fn decode_png(bytes: &[u8], size: Size) -> Result<Image, LoadError> {
    if u64::from(size.width) * u64::from(size.height) > MAX_PIXELS {
        return Err(LoadError::TooLarge);
    }
    let broken = |e: png::DecodingError| LoadError::BadImage(e.to_string());
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // 画素の分は呼び出し側が確保するので、デコーダの中の確保は少しでよい
    decoder.set_limits(png::Limits {
        bytes: 16 * 1024 * 1024,
    });
    let mut reader = decoder.read_info().map_err(broken)?;
    let info = reader.info();
    let found = Size::new(info.width, info.height);
    if found != size {
        return Err(LoadError::SizeMismatch {
            recorded: size,
            image: found,
        });
    }
    if info.color_type != png::ColorType::Rgba
        || info.bit_depth != png::BitDepth::Eight
        || info.interlaced
        || info.is_animated()
    {
        return Err(LoadError::BadImage("not an 8-bit RGBA image".into()));
    }
    let expected = size.width as usize * size.height as usize * 4;
    if reader.output_buffer_size() != Some(expected) {
        return Err(LoadError::BadImage("unexpected image layout".into()));
    }
    let mut pixels = vec![0; expected];
    let frame = reader.next_frame(&mut pixels).map_err(broken)?;
    if frame.buffer_size() != expected {
        return Err(LoadError::BadImage("unexpected image layout".into()));
    }
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Image::from_bgra(size, pixels).ok_or_else(|| LoadError::BadImage("wrong length".into()))
}

/// トリムの範囲。画像 `size` の中の空でない範囲でなければ `OutOfRange`。
fn trim_rect(table: &Table, size: Size) -> Result<Rect, LoadError> {
    only_keys(table, "trim", &["x", "y", "width", "height"])?;
    let x = integer(table, "trim", "x")?;
    let y = integer(table, "trim", "y")?;
    let width = integer(table, "trim", "width")?;
    let height = integer(table, "trim", "height")?;
    let inside = |from: i64, length: i64, limit: u32| {
        from >= 0 && length >= 1 && from + length <= i64::from(limit)
    };
    // 各値は i64 の全体を取りうるので、足す前に画像の一辺で抑える
    let small = |n: i64| (-1..=i64::from(MAX_SIDE)).contains(&n);
    let fits = [x, y, width, height].into_iter().all(small)
        && inside(x, width, size.width)
        && inside(y, height, size.height);
    if !fits {
        return Err(LoadError::OutOfRange(
            "the trim is outside the image".into(),
        ));
    }
    Ok(Rect::new(
        x as i32,
        y as i32,
        (x + width) as i32,
        (y + height) as i32,
    ))
}

fn read_annotation(table: &Table, index: usize) -> Result<Annotation, LoadError> {
    let path = format!("annotations[{index}]");
    let shape_name = table
        .get("shape")
        .and_then(Item::as_str)
        .ok_or_else(|| bad(format!("{path}.shape is missing or not a string")))?;
    let point = |key: &str| -> Result<Point, LoadError> {
        let value = table
            .get(key)
            .and_then(Item::as_value)
            .ok_or_else(|| bad(format!("{path}.{key} is missing")))?;
        point_of(value, &path, key)
    };
    let ends = |make: fn(Point, Point) -> Shape| -> Result<Shape, LoadError> {
        only_keys(table, &path, &["shape", "from", "to", "color", "width"])?;
        Ok(make(point("from")?, point("to")?))
    };
    let shape = match shape_name {
        "rect" => ends(|from, to| Shape::Rect { from, to })?,
        "ellipse" => ends(|from, to| Shape::Ellipse { from, to })?,
        "arrow" => ends(|from, to| Shape::Arrow { from, to })?,
        "line" => ends(|from, to| Shape::Line { from, to })?,
        "freehand" => {
            only_keys(table, &path, &["shape", "points", "color", "width"])?;
            let array = table
                .get("points")
                .and_then(Item::as_array)
                .ok_or_else(|| bad(format!("{path}.points is missing or not an array")))?;
            if array.len() < 2 {
                return Err(bad(format!("{path}.points needs at least 2 points")));
            }
            let points = array
                .iter()
                .map(|value| point_of(value, &path, "points"))
                .collect::<Result<Vec<_>, _>>()?;
            Shape::Freehand { points }
        }
        "text" => {
            only_keys(
                table,
                &path,
                &["shape", "at", "text", "font", "font_size", "color", "width"],
            )?;
            let text = string(table, &path, "text")?;
            if text.chars().count() > MAX_TEXT_CHARS {
                return Err(LoadError::TooLarge);
            }
            let family = string(table, &path, "font")?;
            if family.is_empty()
                || family.len() > MAX_FONT_FAMILY_BYTES
                || family.chars().any(char::is_control)
            {
                return Err(bad(format!("{path}.font is not a font name")));
            }
            let size = integer(table, &path, "font_size")?;
            let size = u32::try_from(size)
                .ok()
                .filter(|s| ANNOTATION_FONT_SIZE_RANGE.contains(s))
                .ok_or_else(|| LoadError::OutOfRange(format!("{path}.font_size")))?;
            Shape::Text {
                at: point("at")?,
                text: text.to_owned(),
                font: Font {
                    family: family.to_owned(),
                    size,
                },
            }
        }
        other => return Err(bad(format!("{path}.shape {other:?} is unknown"))),
    };
    let color: Rgb = string(table, &path, "color")?
        .parse()
        .map_err(|_| bad(format!("{path}.color is not #RRGGBB")))?;
    let width = integer(table, &path, "width")?;
    let width = u32::try_from(width)
        .ok()
        .filter(|w| ANNOTATION_WIDTH_RANGE.contains(w))
        .ok_or_else(|| LoadError::OutOfRange(format!("{path}.width")))?;
    Ok(Annotation {
        shape,
        style: Style { color, width },
    })
}

fn string<'a>(table: &'a Table, path: &str, key: &str) -> Result<&'a str, LoadError> {
    table
        .get(key)
        .and_then(Item::as_str)
        .ok_or_else(|| bad(format!("{path}.{key} is missing or not a string")))
}

/// `[x, y]` の点。座標の絶対値が `MAX_COORDINATE` を超えたら `OutOfRange`。
fn point_of(value: &Value, path: &str, key: &str) -> Result<Point, LoadError> {
    let not_point = || bad(format!("{path}.{key} is not [x, y]"));
    let array = value.as_array().ok_or_else(not_point)?;
    if array.len() != 2 {
        return Err(not_point());
    }
    let coordinate = |i: usize| -> Result<i32, LoadError> {
        let n = array
            .get(i)
            .and_then(Value::as_integer)
            .ok_or_else(not_point)?;
        if !(-MAX_COORDINATE..=MAX_COORDINATE).contains(&n) {
            return Err(LoadError::OutOfRange(format!("{path}.{key}")));
        }
        Ok(n as i32)
    };
    Ok(Point::new(coordinate(0)?, coordinate(1)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrap::{Appearance, AppearanceOverrides, Mode};

    fn image() -> Image {
        // 4×3。透明と半透明の画素も入れる
        let mut pixels = Vec::new();
        for i in 0..12u8 {
            pixels.extend_from_slice(&[i, 20 + i, 40 + i, if i % 3 == 0 { 0 } else { 128 + i }]);
        }
        Image::from_bgra(Size::new(4, 3), pixels).unwrap()
    }

    fn style(width: u32) -> Style {
        Style {
            color: Rgb(0x12, 0xAB, 0xEF),
            width,
        }
    }

    /// 全部の形と文字を持つ書き込み。画像の外に出たものも入れる。
    fn every_shape() -> Vec<Annotation> {
        let p = Point::new;
        vec![
            Annotation {
                shape: Shape::Rect {
                    from: p(3, 2),
                    to: p(0, 0),
                },
                style: style(2),
            },
            Annotation {
                shape: Shape::Ellipse {
                    from: p(-5, -6),
                    to: p(10, 20),
                },
                style: style(3),
            },
            Annotation {
                shape: Shape::Arrow {
                    from: p(1, 1),
                    to: p(2, 2),
                },
                style: style(5),
            },
            Annotation {
                shape: Shape::Line {
                    from: p(0, 2),
                    to: p(3, 0),
                },
                style: style(8),
            },
            Annotation {
                shape: Shape::Freehand {
                    points: vec![p(0, 0), p(1, 2), p(3, 1)],
                },
                style: style(64),
            },
            Annotation {
                shape: Shape::Text {
                    at: p(1, 1),
                    text: "二行の\n\"文字\" \\ #[x]".into(),
                    font: Font {
                        family: "Yu Gothic UI".into(),
                        size: 24,
                    },
                },
                style: style(3),
            },
        ]
    }

    fn sample() -> ScrapFile {
        ScrapFile {
            content: Content::Image(image()),
            trim: Some(Rect::new(1, 1, 3, 3)),
            annotations: every_shape(),
        }
    }

    /// 識別子と版と記録と後ろのバイトから、ファイルのバイト列を組む。
    fn assemble(version: u32, record: &str, payload: &[u8]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&(record.len() as u32).to_le_bytes());
        out.extend_from_slice(record.as_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn image_record(width: u32, height: u32) -> String {
        format!("[source]\nkind = \"image\"\nwidth = {width}\nheight = {height}\n")
    }

    fn png() -> Vec<u8> {
        image().to_png().unwrap()
    }

    #[test]
    fn an_image_scrap_with_every_shape_and_a_trim_comes_back_as_it_was() {
        let file = sample();
        let bytes = encode(&file).unwrap();
        assert!(bytes.starts_with(MAGIC));
        assert_eq!(decode(&bytes).unwrap(), file);
    }

    fn blank(background: Background) -> ScrapFile {
        ScrapFile {
            content: Content::Blank {
                size: Size::new(4, 3),
                background,
            },
            ..sample()
        }
    }

    #[test]
    fn a_blank_scrap_of_a_color_or_transparent_comes_back_as_it_was_without_a_png() {
        for background in [
            Background::Color(Rgb(0x12, 0x34, 0x56)),
            Background::Transparent,
        ] {
            let file = blank(background);
            let bytes = encode(&file).unwrap();
            assert_eq!(decode(&bytes).unwrap(), file);
            // 記録で終わり、後ろに PNG を持たない
            let record_length = u32::from_le_bytes(bytes[17..21].try_into().unwrap()) as usize;
            assert_eq!(bytes.len(), MAGIC.len() + HEADER_NUMBERS + record_length);
        }
    }

    #[test]
    fn a_file_whose_blankness_disagrees_with_having_an_image_is_malformed() {
        let blank_record =
            "[source]\nkind = \"blank\"\nwidth = 4\nheight = 3\nbackground = \"#FFFFFF\"\n";
        let cases = [
            // 白紙なのに PNG が続く
            assemble(VERSION, blank_record, &png()),
            // 白紙なのに後ろに何か残る
            assemble(VERSION, blank_record, b"x"),
            // 画像なのに背景色を持つ
            assemble(
                VERSION,
                &format!("{}background = \"#FFFFFF\"\n", image_record(4, 3)),
                &png(),
            ),
            // 画像なのに PNG がない
            assemble(VERSION, &image_record(4, 3), &[]),
        ];
        for (i, bytes) in cases.iter().enumerate() {
            let error = decode(bytes).unwrap_err();
            assert_eq!(
                error.kind(),
                LoadErrorKind::Malformed,
                "case {i}: {error:?}"
            );
        }
    }

    #[test]
    fn a_blank_record_needs_a_background_that_is_a_color_or_transparent_and_a_size_in_the_limits() {
        let record = |background: &str, width: u32, height: u32| {
            format!("[source]\nkind = \"blank\"\nwidth = {width}\nheight = {height}\n{background}")
        };
        for (i, text) in [
            record("", 4, 3),
            record("background = \"Transparent\"\n", 4, 3),
            record("background = \"#FFF\"\n", 4, 3),
            record("background = 0\n", 4, 3),
            record("background = \"#FFFFFF\"\n", 0, 3),
        ]
        .iter()
        .enumerate()
        {
            let error = decode(&assemble(VERSION, text, &[])).unwrap_err();
            assert!(
                matches!(error, LoadError::BadRecord(_)),
                "case {i}: {error:?}"
            );
        }
        let huge = record("background = \"#FFFFFF\"\n", MAX_SIDE, MAX_SIDE);
        assert!(matches!(
            decode(&assemble(VERSION, &huge, &[])),
            Err(LoadError::TooLarge)
        ));
        let outside = format!(
            "{}[trim]\nx = 3\ny = 0\nwidth = 2\nheight = 1\n",
            record("background = \"#FFFFFF\"\n", 4, 3)
        );
        assert!(matches!(
            decode(&assemble(VERSION, &outside, &[])),
            Err(LoadError::OutOfRange(_))
        ));
    }

    #[test]
    fn saving_a_blank_scrap_takes_its_background_and_size_instead_of_the_painted_image() {
        for background in [Background::WHITE, Background::Transparent] {
            let model = ScrapModel::restored(
                Point::new(5, 6),
                Size::new(4, 3),
                Some(background),
                Some(Rect::new(1, 1, 3, 3)),
                Annotations::from_items(every_shape()),
            );
            let painted = model.blank_image().unwrap();
            assert_eq!(ScrapFile::of(&model, &painted), blank(background));
        }
    }

    #[test]
    fn an_opened_blank_scrap_keeps_its_background_and_can_change_it() {
        let monitor = Rect::new(0, 0, 100, 100);
        let (mut model, content) = blank(Background::Transparent).into_scrap(monitor);
        assert_eq!(
            content,
            Content::Blank {
                size: Size::new(4, 3),
                background: Background::Transparent
            }
        );
        assert_eq!(model.background(), Some(Background::Transparent));
        assert_eq!(model.image_rect(), Rect::new(49, 49, 51, 51));
        assert_eq!(model.annotations().items(), every_shape().as_slice());
        assert!(!model.is_empty_transparent_blank());
        // 開いた白紙スクラップでも背景色を変えられ、下地はその色で塗られる
        assert!(model.set_background(Background::WHITE));
        assert_eq!(
            model.blank_image(),
            Image::from_bgra(Size::new(4, 3), [0xFF; 4].repeat(12))
        );
        // 何も書かれていない透明な白紙は、開いても捨てる決まりの対象
        let empty = ScrapFile {
            annotations: vec![],
            ..blank(Background::Transparent)
        };
        assert!(empty.into_scrap(monitor).0.is_empty_transparent_blank());
    }

    #[test]
    fn a_scrap_without_trim_or_annotations_comes_back_as_it_was() {
        let file = ScrapFile {
            trim: None,
            annotations: vec![],
            ..sample()
        };
        assert_eq!(decode(&encode(&file).unwrap()).unwrap(), file);
    }

    #[test]
    fn saving_takes_the_original_image_the_trim_and_the_annotations_of_the_scrap() {
        let model = ScrapModel::restored(
            Point::new(500, 600),
            Size::new(4, 3),
            None,
            Some(Rect::new(1, 1, 3, 3)),
            Annotations::from_items(every_shape()),
        );
        assert_eq!(ScrapFile::of(&model, &image()), sample());
    }

    #[test]
    fn an_opened_scrap_sits_at_the_monitor_center_at_100_percent_with_default_looks_and_no_history()
    {
        let monitor = Rect::new(-1920, 0, 0, 1080);
        let (model, content) = sample().into_scrap(monitor);
        assert_eq!(content, Content::Image(image()));
        // 見えている 2×2 の中心がモニタの中心
        assert_eq!(model.image_rect(), Rect::new(-961, 539, -959, 541));
        assert_eq!(model.zoom(), 100);
        assert_eq!(model.mode, Mode::Normal);
        assert!(!model.is_compact());
        assert_eq!(model.appearance_overrides, AppearanceOverrides::default());
        assert_eq!(
            model.appearance(&Appearance::default()),
            Appearance::default()
        );
        assert_eq!(model.annotations().items(), every_shape().as_slice());
        assert!(!model.annotations().can_undo());
        assert!(!model.annotations().can_redo());
    }

    #[test]
    fn untrimming_an_opened_scrap_brings_back_the_hidden_part_and_keeps_the_annotations() {
        let (mut model, _) = sample().into_scrap(Rect::new(0, 0, 100, 100));
        assert!(model.untrim());
        assert_eq!(model.shown_size(), Size::new(4, 3));
        // トリムした部分は画面上の同じ位置に残る
        assert_eq!(model.image_rect(), Rect::new(48, 48, 52, 51));
        assert_eq!(model.annotations().items(), every_shape().as_slice());
    }

    #[test]
    fn a_file_that_does_not_start_with_the_magic_is_not_a_scrap_file() {
        for bytes in [png(), b"\x89SHUNSOK".to_vec(), vec![]] {
            assert!(matches!(decode(&bytes), Err(LoadError::NotScrapFile)));
        }
        let mut bytes = encode(&sample()).unwrap();
        bytes[5] = b'X';
        let error = decode(&bytes).unwrap_err();
        assert!(matches!(error, LoadError::NotScrapFile));
        assert_eq!(error.kind(), LoadErrorKind::Malformed);
    }

    #[test]
    fn a_file_from_a_newer_version_is_refused_as_such() {
        let bytes = assemble(VERSION + 1, "whatever = 1", b"anything");
        let error = decode(&bytes).unwrap_err();
        assert!(matches!(error, LoadError::NewerVersion(2)));
        assert_eq!(error.kind(), LoadErrorKind::NewerVersion);
    }

    #[test]
    fn a_broken_record_is_refused() {
        let record = image_record(4, 3);
        let annotation = |body: &str| format!("{record}[[annotations]]\n{body}\n");
        let cases = [
            // TOML として読めない
            assemble(VERSION, "[source\nkind = ", &png()),
            // 版 0
            assemble(0, &record, &png()),
            // 記録が途中で切れている
            {
                let mut bytes = assemble(VERSION, &record, &[]);
                bytes.truncate(bytes.len() - 3);
                bytes
            },
            // 版の途中で切れている
            MAGIC.iter().copied().chain([1, 0]).collect(),
            // [source] がない、知らない種類
            assemble(VERSION, "", &png()),
            assemble(
                VERSION,
                "[source]\nkind = \"video\"\nwidth = 4\nheight = 3\n",
                &png(),
            ),
            // 知らないキー
            assemble(VERSION, &format!("{record}zoom = 200\n"), &png()),
            assemble(VERSION, &format!("x = 1\n{record}"), &png()),
            // 型が違う、大きさが 0
            assemble(
                VERSION,
                "[source]\nkind = \"image\"\nwidth = \"4\"\nheight = 3\n",
                &png(),
            ),
            assemble(VERSION, &image_record(0, 3), &png()),
            // 知らない形、色が #RRGGBB でない、点が 1 つのフリーハンド、知らないキー
            assemble(
                VERSION,
                &annotation("shape = \"star\"\ncolor = \"#000000\"\nwidth = 3"),
                &png(),
            ),
            assemble(
                VERSION,
                &annotation(
                    "shape = \"line\"\nfrom = [0, 0]\nto = [1, 1]\ncolor = \"red\"\nwidth = 3",
                ),
                &png(),
            ),
            assemble(
                VERSION,
                &annotation(
                    "shape = \"freehand\"\npoints = [[0, 0]]\ncolor = \"#000000\"\nwidth = 3",
                ),
                &png(),
            ),
            assemble(
                VERSION,
                &annotation(
                    "shape = \"line\"\nfrom = [0, 0]\nto = [1, 1, 2]\ncolor = \"#000000\"\nwidth = 3",
                ),
                &png(),
            ),
            assemble(
                VERSION,
                &annotation(
                    "shape = \"line\"\nfrom = [0, 0]\nto = [1, 1]\npoints = []\ncolor = \"#000000\"\nwidth = 3",
                ),
                &png(),
            ),
        ];
        for (i, bytes) in cases.iter().enumerate() {
            let error = decode(bytes).unwrap_err();
            assert!(
                matches!(error, LoadError::BadRecord(_)),
                "case {i}: {error:?}"
            );
            assert_eq!(error.kind(), LoadErrorKind::Malformed);
        }
    }

    #[test]
    fn a_broken_png_is_refused() {
        let record = image_record(4, 3);
        let mut damaged = png();
        let at = damaged.len() - 20;
        damaged[at] ^= 0xFF;
        let gray = {
            let mut out = Vec::new();
            let mut encoder = png::Encoder::new(&mut out, 4, 3);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0; 12]).unwrap();
            writer.finish().unwrap();
            out
        };
        for (i, payload) in [vec![], b"not a png".to_vec(), damaged, gray]
            .iter()
            .enumerate()
        {
            let error = decode(&assemble(VERSION, &record, payload)).unwrap_err();
            assert!(
                matches!(error, LoadError::BadImage(_)),
                "case {i}: {error:?}"
            );
            assert_eq!(error.kind(), LoadErrorKind::Malformed);
        }
    }

    #[test]
    fn a_png_of_another_size_than_the_record_is_refused() {
        let error = decode(&assemble(VERSION, &image_record(5, 3), &png())).unwrap_err();
        assert!(
            matches!(
                error,
                LoadError::SizeMismatch {
                    recorded: Size {
                        width: 5,
                        height: 3
                    },
                    image: Size {
                        width: 4,
                        height: 3
                    },
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn sizes_over_the_limits_are_refused_before_reading_the_image() {
        let cases = [
            // 一辺が上限を超える
            assemble(VERSION, &image_record(MAX_SIDE + 1, 1), &png()),
            // 画素の数が上限を超える
            assemble(VERSION, &image_record(MAX_SIDE, MAX_SIDE), &png()),
            // 記録が上限を超える
            {
                let mut bytes = MAGIC.to_vec();
                bytes.extend_from_slice(&VERSION.to_le_bytes());
                bytes.extend_from_slice(&(MAX_RECORD_BYTES as u32 + 1).to_le_bytes());
                bytes
            },
            // ファイルが上限を超える
            vec![0; MAX_FILE_BYTES as usize + 1],
        ];
        for (i, bytes) in cases.iter().enumerate() {
            let error = decode(bytes).unwrap_err();
            assert!(matches!(error, LoadError::TooLarge), "case {i}: {error:?}");
            assert_eq!(error.kind(), LoadErrorKind::Malformed);
        }
    }

    #[test]
    fn a_trim_or_annotation_values_outside_the_allowed_range_are_refused() {
        let record = image_record(4, 3);
        let trims = [
            "x = 0\ny = 0\nwidth = 5\nheight = 3",
            "x = -1\ny = 0\nwidth = 2\nheight = 2",
            "x = 0\ny = 0\nwidth = 0\nheight = 2",
            "x = 9223372036854775807\ny = 0\nwidth = 9223372036854775807\nheight = 2",
        ];
        let annotations = [
            "shape = \"line\"\nfrom = [0, 0]\nto = [-9223372036854775808, 0]\ncolor = \"#000000\"\nwidth = 3",
            "shape = \"line\"\nfrom = [0, 0]\nto = [1, 1]\ncolor = \"#000000\"\nwidth = 65",
            "shape = \"line\"\nfrom = [0, 0]\nto = [1, 1]\ncolor = \"#000000\"\nwidth = 0",
            "shape = \"text\"\nat = [0, 0]\ntext = \"a\"\nfont = \"x\"\nfont_size = 257\ncolor = \"#000000\"\nwidth = 3",
        ];
        let records = trims
            .iter()
            .map(|t| format!("{record}[trim]\n{t}\n"))
            .chain(
                annotations
                    .iter()
                    .map(|a| format!("{record}[[annotations]]\n{a}\n")),
            );
        for (i, record) in records.enumerate() {
            let error = decode(&assemble(VERSION, &record, &png())).unwrap_err();
            assert!(
                matches!(error, LoadError::OutOfRange(_)),
                "case {i}: {error:?}"
            );
        }
    }

    #[test]
    fn a_missing_file_is_unreadable() {
        let error = read(Path::new("/nonexistent/shunsoku/scrap.shunsoku")).unwrap_err();
        assert_eq!(error.kind(), LoadErrorKind::Unreadable);
    }

    #[test]
    fn something_other_than_a_plain_file_is_unreadable_without_reading_it() {
        // 読みに行くと上限まで読んでしまう（デバイスやパイプを CLI から渡されても読まない）
        #[cfg(unix)]
        let device = Path::new("/dev/zero");
        #[cfg(windows)]
        let device = Path::new(r"C:\Windows");
        let error = read(device).unwrap_err();
        assert_eq!(error.kind(), LoadErrorKind::Unreadable);
    }

    #[test]
    fn every_reason_has_its_own_message_in_each_language() {
        let errors = [
            LoadError::Unreadable(std::io::Error::other("x")),
            LoadError::NotScrapFile,
            LoadError::NewerVersion(2),
            LoadError::BadRecord(String::new()),
            LoadError::BadImage(String::new()),
            LoadError::SizeMismatch {
                recorded: Size::new(1, 1),
                image: Size::new(2, 2),
            },
            LoadError::TooLarge,
            LoadError::OutOfRange(String::new()),
        ];
        for source in [
            include_str!("../../../locales/ja.toml"),
            include_str!("../../../locales/en.toml"),
        ] {
            let catalog = crate::i18n::Catalog::parse(source).unwrap();
            let mut seen = Vec::new();
            for error in &errors {
                let key = error.message_key();
                assert_ne!(catalog.text(key), key, "{key}");
                assert!(!seen.contains(&key), "{key}");
                seen.push(key);
            }
            assert_ne!(catalog.text("scrap_file.filter"), "scrap_file.filter");
        }
    }
}
