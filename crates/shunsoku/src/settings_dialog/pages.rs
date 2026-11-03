//! 「一般」「スクラップ」「書き込み」のタブ。行は上から `y` を進めて置く。

use super::fields::{Field, TemplateOf};
use super::{FIELD_X, LABEL_WIDTH, PAGE_WIDTH, PageBuilder};

/// 行の間隔（欄 1 行）。
const ROW: i16 = 20;
/// 欄の幅（ページの右端まで）。
const FIELD_WIDTH: i16 = PAGE_WIDTH - FIELD_X;

/// 「一般」タブ。言語を先頭に置く。自動起動（#101）の欄は、言語の後ろ（通知の前）に `y` を進めて足す。
pub(super) fn general(b: &mut PageBuilder) {
    let mut y = 0;
    b.label("settings_gui.general.language", 0, y + 2, LABEL_WIDTH, 10);
    b.choice(Field::Language, FIELD_X, y, 100, 60);
    y += ROW;
    b.check(
        Field::Notifications,
        "settings_gui.general.notifications",
        0,
        y,
        PAGE_WIDTH,
    );
    y += ROW;
    b.label("settings_gui.general.save_dir", 0, y + 2, LABEL_WIDTH, 10);
    b.folder_field(Field::SaveDir, FIELD_X, y, FIELD_WIDTH);
    y += ROW;
    b.label(
        "settings_gui.general.file_name_template",
        0,
        y + 2,
        LABEL_WIDTH,
        10,
    );
    b.template_field(TemplateOf::General, FIELD_X, y, FIELD_WIDTH);
}

/// 「スクラップ」タブ。
pub(super) fn scrap(b: &mut PageBuilder) {
    let mut y = 0;
    b.label("settings_gui.scrap.opacity", 0, y + 2, LABEL_WIDTH, 10);
    b.number(Field::Opacity, FIELD_X, y, 40);
    y += ROW;
    b.check(Field::Border, "settings_gui.scrap.border", 0, y, PAGE_WIDTH);
    y += ROW;
    b.label("settings_gui.scrap.border_width", 0, y + 2, LABEL_WIDTH, 10);
    b.number(Field::BorderWidth, FIELD_X, y, 40);
    y += ROW;
    b.label("settings_gui.scrap.border_color", 0, y + 2, LABEL_WIDTH, 10);
    b.color_field(Field::BorderColor, FIELD_X, y);
    y += ROW;
    b.check(Field::Shadow, "settings_gui.scrap.shadow", 0, y, PAGE_WIDTH);
    y += ROW;
    b.label("settings_gui.scrap.trash_limit", 0, y + 2, LABEL_WIDTH, 10);
    b.number(Field::TrashLimit, FIELD_X, y, 40);
    y += ROW;
    b.label("settings_gui.scrap.dpi_scaling", 0, y + 2, LABEL_WIDTH, 10);
    b.choice(Field::DpiScaling, FIELD_X, y, FIELD_WIDTH, 60);
}

/// 「書き込み」タブ。
pub(super) fn annotation(b: &mut PageBuilder) {
    let mut y = 0;
    b.label("settings_gui.annotation.color", 0, y + 2, LABEL_WIDTH, 10);
    b.color_field(Field::AnnotationColor, FIELD_X, y);
    y += ROW;
    b.label("settings_gui.annotation.width", 0, y + 2, LABEL_WIDTH, 10);
    b.choice(Field::AnnotationWidth, FIELD_X, y, 50, 100);
    y += ROW;
    b.label(
        "settings_gui.annotation.font_size",
        0,
        y + 2,
        LABEL_WIDTH,
        10,
    );
    b.choice(Field::AnnotationFontSize, FIELD_X, y, 50, 100);
    y += ROW;
    b.label("settings_gui.annotation.font", 0, y + 2, LABEL_WIDTH, 10);
    b.choice(Field::AnnotationFont, FIELD_X, y, FIELD_WIDTH, 120);
    y += ROW + 4;
    b.label("settings_gui.annotation.hint", 0, y, PAGE_WIDTH, 20);
}
