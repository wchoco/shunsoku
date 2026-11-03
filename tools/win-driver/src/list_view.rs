//! リストビューの文字が列に収まっているかの判定（M5-10 の「文字が切れない」を値で見る）。
//!
//! 一覧は、文字の幅（`LVM_GETSTRINGWIDTHW`）に左右の余白を足した幅が列より広いと、文字を「…」で略す。
//! 余白は見出しと行で違い、行でも先頭の列とそれ以外で違う。

/// 文字の置き場所。余白が違う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Header,
    FirstColumn,
    OtherColumn,
}

impl Place {
    /// 行の文字の、列 `column` での置き場所。
    pub fn of_row(column: usize) -> Place {
        if column == 0 {
            Place::FirstColumn
        } else {
            Place::OtherColumn
        }
    }

    /// `dpi` のときの、左右の余白の合計（ピクセル）。見出しは本体と同じ `app_window::LIST_HEADER_PADDING`（96 DPI で 14）を
    /// DPI で拡大した値。行は、`LVSCW_AUTOSIZE` が付けた幅（略さない最小の幅）と文字の幅の差を実機で測った値で、
    /// 100% でも 150% でも同じだった（DPI で変わらない）。
    pub fn pad(self, dpi: u32) -> i32 {
        match self {
            Place::Header => padding(dpi, shunsoku_core::app_window::LIST_HEADER_PADDING),
            Place::FirstColumn => 8,
            Place::OtherColumn => 12,
        }
    }
}

/// 文字 1 つ。
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub text: String,
    /// 一覧のフォントで測った文字の幅（ピクセル）。
    pub width: i32,
}

/// `dpi` のときの余白。`pad_96` は 96 DPI のときの値。
pub fn padding(dpi: u32, pad_96: i32) -> i32 {
    pad_96 * dpi as i32 / 96
}

/// 幅 `column` の列で、`cell` が略されるか。空の文字は略されない。
pub fn clipped(column: i32, cell: &Cell, pad: i32) -> bool {
    !cell.text.is_empty() && cell.width + pad > column
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(text: &str, width: i32) -> Cell {
        Cell {
            text: text.into(),
            width,
        }
    }

    #[test]
    fn rows_need_less_padding_than_the_header() {
        // 96 DPI で `LVSCW_AUTOSIZE` が付けた幅から測った値（先頭の列 8、それ以外 12）と、本体の見出しの余白 14
        assert_eq!(Place::Header.pad(96), 14);
        assert_eq!(Place::FirstColumn.pad(96), 8);
        assert_eq!(Place::OtherColumn.pad(96), 12);
        assert_eq!(Place::of_row(0), Place::FirstColumn);
        assert_eq!(Place::of_row(2), Place::OtherColumn);
    }

    #[test]
    fn only_the_header_padding_scales_with_dpi() {
        // 150% の実機で、行の余白は 96 DPI と同じ 8 と 12 だった
        assert_eq!(Place::Header.pad(96), 14);
        assert_eq!(Place::Header.pad(144), 21);
        assert_eq!(Place::FirstColumn.pad(144), 8);
        assert_eq!(Place::OtherColumn.pad(144), 12);
    }

    #[test]
    fn text_fits_when_text_and_padding_fit_in_the_column() {
        // 150% の「ホットキー」の見出し: 文字 78 px、余白 21 px
        assert!(!clipped(99, &cell("ホットキー", 78), 21));
        assert!(clipped(98, &cell("ホットキー", 78), 21));
    }

    #[test]
    fn empty_text_is_never_clipped() {
        assert!(!clipped(0, &cell("", 0), 21));
    }
}
