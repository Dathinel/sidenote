//! Colours, fonts and the type scale. Light and dark follow the system setting. `soft` and
//! `faint` keep 4.5:1 on the page, raised surfaces, the sidebar and the hover, selected and
//! highlight fills in both modes. docs/design/typography.md lists the ratios.
use rune::{Color, Family, TextStyle};
use rune_kit::{Kit, PaletteSeed};

#[derive(Clone, Copy)]
pub struct Palette {
    /// The reading page.
    pub bg: Color,
    /// Raised surfaces: the Ask pill, its thread, inputs and menus.
    pub card: Color,
    /// The sidebar.
    pub side: Color,
    pub ink: Color,
    pub soft: Color,
    pub faint: Color,
    pub line: Color,
    pub accent: Color,
    pub ok: Color,
    pub okbg: Color,
    pub warn: Color,
    pub warnbg: Color,
    pub open: Color,
    /// Hover fills, count pills and quiet buttons.
    pub chip: Color,
    /// The selected row in a list: the accent at 10% over the page.
    pub sel: Color,
    pub hit: Color,
    pub shadow: Color,
    /// Dims the page under a modal, dark enough that page headings do not compete with it.
    pub scrim: Color,
    pub button_fill: Color,
    pub button_ink: Color,
    pub button_border: Color,
    pub danger: Color,
    pub danger_border: Color,
    pub button_hover: Color,
    pub button_pressed: Color,
    pub button_primary_hover: Color,
    pub button_primary_pressed: Color,
}

fn hex(value: &str) -> Color {
    rune::color::hex(value).expect("valid theme colour")
}

pub fn palette(dark: bool) -> Palette {
    let c = if dark {
        [
            "#1a1a18",
            "#242422",
            "#141413",
            "#e9e7e1",
            "#bcb9b0",
            "#b0ada4",
            "#302f2c",
            "#8db5dc",
            "#69c995",
            "#1b2c22",
            "#e3aa55",
            "#33281a",
            "#b49ee3",
            "#2a2a27",
            "#262a2c",
            "#363019",
            "#00000066",
            "#000000b3",
        ]
    } else {
        [
            "#fcfbf8",
            "#ffffff",
            "#f3f2ed",
            "#1e1e1c",
            "#5d5b55",
            "#6b6861",
            "#e6e3db",
            "#2f5d8a",
            "#1d7a4c",
            "#e4f3eb",
            "#9a5800",
            "#fcf0dc",
            "#6a4ea3",
            "#efede7",
            "#e8ebed",
            "#fff3c4",
            "#1e1e1c1f",
            "#1e1e1c73",
        ]
    }
    .map(|s| rune::color::hex(s).expect("palette colours are valid hex"));
    Palette {
        bg: c[0],
        card: c[1],
        side: c[2],
        ink: c[3],
        soft: c[4],
        faint: c[5],
        line: c[6],
        accent: c[7],
        ok: c[8],
        okbg: c[9],
        warn: c[10],
        warnbg: c[11],
        open: c[12],
        chip: c[13],
        sel: c[14],
        hit: c[15],
        shadow: c[16],
        scrim: c[17],
        button_fill: hex(if dark { "#ececec" } else { "#1f2328" }),
        button_ink: hex(if dark { "#1f2328" } else { "#ffffff" }),
        button_border: if dark { c[6] } else { hex("#dcd9d2") },
        danger: hex(if dark { "#f29b99" } else { "#a92b30" }),
        danger_border: hex(if dark { "#704345" } else { "#efc5c5" }),
        button_hover: hex(if dark { "#ffffff0f" } else { "#00000008" }),
        button_pressed: hex(if dark { "#ffffff1f" } else { "#00000014" }),
        button_primary_hover: hex(if dark { "#0000000f" } else { "#ffffff14" }),
        button_primary_pressed: hex(if dark { "#0000001f" } else { "#ffffff24" }),
    }
}

/// Kit controls (text fields, the runner dropdown, Markdown) drawn from the same palette.
pub fn kit(p: &Palette) -> Kit {
    let mut kit = Kit::from_palette(PaletteSeed {
        bg: p.card,
        fg: p.ink,
        accent: p.accent,
    });
    kit.glyph = rune_kit::Glyphs {
        check: &super::icons::CHECK,
        check_small: &super::icons::CHECK,
        chevron_down: &super::icons::CHEVRON_DOWN,
        chevron_up: &super::icons::CHEVRON_UP,
        caret_right: &super::icons::CHEVRON_RIGHT,
        triangle_down: &super::icons::CHEVRON_DOWN,
        triangle_right: &super::icons::CHEVRON_RIGHT,
        close: &super::icons::X,
        clear: &super::icons::X,
        copy: &super::icons::COPY,
        search: &super::icons::SEARCH,
        lock: &super::icons::LOCK,
        eye: &super::icons::EYE,
        eye_off: &super::icons::EYE_OFF,
        circle_x: &super::icons::CIRCLE_X,
        dash: &super::icons::DASH,
    };
    kit.c.bg_accent = p.button_fill;
    kit.c.knob_on = p.button_ink;
    kit.c.text_faint = p.faint;
    kit.ty.small = super::roles::META.style();
    kit
}

/// Reading text: summary points, to-dos and answers.
pub const BODY: f32 = 16.0;
pub const BODY_LH: f32 = 27.0;

/// The serif used for page titles. Windows ships Georgia; without it titles fall back to Inter.
const DISPLAY: Family = Family::Named("Georgia");

pub fn display(size: f32) -> TextStyle {
    TextStyle::new(size, 400.0)
        .family(DISPLAY)
        .lh((size * 1.25).round())
}

/// Inter as before, plus Georgia from the Windows fonts folder for titles. Rune replaces its
/// built-in Inter when an app passes any fonts, so Inter is bundled here too.
pub fn fonts() -> &'static [&'static [u8]] {
    let mut list: Vec<&'static [u8]> = vec![
        include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
        include_bytes!("../../assets/fonts/Inter-Medium.ttf"),
        include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
    ];
    let dir = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into())
        .join("Fonts");
    if let Ok(bytes) = std::fs::read(dir.join("georgia.ttf")) {
        // Read once at startup and kept for the life of the process, as Rune requires.
        list.push(Box::leak(bytes.into_boxed_slice()));
    }
    Box::leak(list.into_boxed_slice())
}

#[cfg(test)]
mod quiet_ink_switch_tests {
    use super::*;
    #[test]
    fn switches_use_primary_button_ink_in_both_themes() {
        for dark in [false, true] {
            let p = palette(dark);
            let k = kit(&p);
            assert_eq!(k.c.bg_accent, p.button_fill);
            assert_eq!(k.c.knob_on, p.button_ink);
            assert_ne!(k.c.bg_accent, p.accent);
        }
    }
}
