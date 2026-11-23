//! Shared Settings and page list geometry.
use super::{icons, roles, theme::Palette};
use rune::{El, Icon, col, row};

pub fn panel(p: Palette) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .padding(1.0)
        .radius(10.0)
        .clip_rounded(10.0)
        .border(1.0, p.line)
        .fill(p.card)
}

pub fn icon_tile(icon: &'static Icon, p: Palette) -> El {
    col()
        .w(34.0)
        .h(34.0)
        .center()
        .radius(8.0)
        .fill(p.chip)
        .child(icons::draw(icon, 18.0, p.soft))
}

pub fn row_shell() -> El {
    row().w_full().items_center().gaps(12.0, 10.0).padding(14.0)
}

const ROW_PADDING: f32 = 14.0;
const TILE: f32 = 28.0;
const ROW_GAP: f32 = 12.0;

pub fn settings_row(has_help: bool) -> El {
    row()
        .w_full()
        .items_center()
        .flex_wrap()
        .gap(ROW_GAP)
        .px(ROW_PADDING)
        .py(if has_help { 9.0 } else { 7.0 })
        .min_h(if has_help { 56.0 } else { 44.0 })
}

pub fn settings_tile(icon: &'static Icon, p: Palette) -> El {
    col()
        .size(TILE, TILE)
        .center()
        .radius(7.0)
        .fill(p.chip)
        .child(icons::draw(icon, icons::ROW, p.soft))
}

pub fn settings_copy(name: &str, p: Palette) -> El {
    settings_copy_with_title(settings_title(name, p))
}

pub fn settings_title(name: &str, p: Palette) -> El {
    rune::text(name)
        .font(roles::NAV.style().lh(19.0))
        .color(p.ink)
        .wrap()
}

pub fn settings_copy_with_title(title: El) -> El {
    col()
        .grow(1.0)
        .min_w(120.0)
        .gap(roles::META_GAP)
        .child(title)
}

pub fn settings_divider(p: Palette) -> El {
    rune::el()
        .h(1.0)
        .ml(ROW_PADDING + TILE + ROW_GAP)
        .fill(p.line)
}

pub fn copy(title: El, help: &str, p: Palette) -> El {
    col()
        .grow(1.0)
        .min_w(170.0)
        .gap(4.0)
        .child(title)
        .child(roles::SMALL.wrap(help, p.soft))
}

/// The text column of a row whose lines the caller adds. It may shrink to nothing, so an
/// ellipsis name never pushes the row wider than its card.
pub fn copy_column() -> El {
    col().grow(1.0).min_w(0.0).gap(4.0)
}

pub fn fold_control(title: &str, key: &str, open: bool, p: Palette) -> El {
    super::widgets::pressable(
        row()
            .w_full()
            .items_center()
            .gap(8.0)
            .py(12.0)
            .child(icons::draw(
                if open {
                    &icons::CHEVRON_DOWN
                } else {
                    &icons::CHEVRON_RIGHT
                },
                icons::INLINE,
                p.soft,
            ))
            .child(roles::NAME.line(title, p.ink)),
    )
    .key(key)
    .label(title)
    .role("button")
    .expanded(open)
}
