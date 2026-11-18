//! Colours, fonts and the type scale. Light and dark follow the system setting. Greys carry
//! structure and hierarchy (`bg`, `card`, `side`, `line`, then the text greys `ink`, `soft`,
//! `faint`); the accent marks the one main action and the focus ring; green, amber and
//! red mean status only. Every text token keeps 4.5:1 on the fills it sits on, in both modes
//! (`text_tokens_keep_four_and_a_half_to_one`). docs/design/typography.md lists the ratios.
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
    /// The dimmest text grey: facts, placeholders and small notes. Still 4.5:1 on every fill,
    /// and in dark mode never below #a9a69f on a raised surface.
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
    kit.c.knob = Color::WHITE;
    kit.c.text_faint = p.faint;
    kit.ty.small = super::roles::META.style();
    kit
}

/// Reading text: summary points, to-dos and answers (14.5 px, line height 1.6).
pub const BODY: f32 = 14.5;
pub const BODY_LH: f32 = 23.0;

/// The largest a page title gets. Every page title goes through `display`, so one number sets
/// them all.
pub const PAGE_TITLE: f32 = 26.0;

/// Titles use bundled Fraunces on every platform, so Windows and Linux draw the same face.
const DISPLAY: Family = Family::Named("Fraunces");

pub fn display(size: f32) -> TextStyle {
    let size = size.min(PAGE_TITLE);
    TextStyle::new(size, 400.0)
        .family(DISPLAY)
        .lh((size * 1.25).round())
}

/// The sans list. Figtree draws Latin, with Inter behind it. Windows ships Nirmala UI, which
/// draws Telugu and Devanagari, so text in those scripts, such as the Telugu examples in the
/// translation prompt, shows letters and not empty boxes. Linux bundles Noto Sans Devanagari
/// and Noto Sans Telugu for the same job. Rune has no Telugu fallback of its own.
#[cfg(windows)]
pub const SANS: &str = "Figtree, Inter, Nirmala UI";
#[cfg(not(windows))]
pub const SANS: &str = "Figtree, Inter, Noto Sans Devanagari, Noto Sans Telugu";

/// The bundled faces every platform loads: Figtree for text, Fraunces for titles and Inter
/// as the mono slot's name and a Latin fallback.
fn bundled() -> Vec<&'static [u8]> {
    vec![
        include_bytes!("../../assets/fonts/Figtree-Variable.ttf"),
        include_bytes!("../../assets/fonts/Fraunces-Variable.ttf"),
        include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
        include_bytes!("../../assets/fonts/Inter-Medium.ttf"),
        include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
    ]
}

/// Linux uses only the repo's licensed fonts, with Noto for Devanagari and Telugu.
#[cfg(not(windows))]
pub fn fonts() -> &'static [&'static [u8]] {
    let mut list = bundled();
    list.extend([
        include_bytes!("../../assets/fonts/NotoSansDevanagari-Variable.ttf").as_slice(),
        include_bytes!("../../assets/fonts/NotoSansTelugu-Variable.ttf").as_slice(),
    ]);
    Box::leak(list.into_boxed_slice())
}

/// The bundled faces, plus Nirmala UI from Windows for the scripts they lack.
#[cfg(windows)]
pub fn fonts() -> &'static [&'static [u8]] {
    let mut list = bundled();
    let dir = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into())
        .join("Fonts");
    if let Ok(bytes) = std::fs::read(dir.join("nirmala.ttc")) {
        // Read once at startup and kept for the life of the process, as Rune requires.
        list.push(Box::leak(bytes.into_boxed_slice()));
    }
    Box::leak(list.into_boxed_slice())
}

#[cfg(test)]
mod quiet_ink_switch_tests {
    use super::*;

    #[test]
    fn the_sans_list_leads_with_figtree_and_names_the_indic_fallback() {
        assert!(SANS.starts_with("Figtree"));
        assert_eq!(SANS.contains("Nirmala UI"), cfg!(windows));
    }

    #[test]
    fn off_switches_keep_a_light_knob_in_both_themes() {
        for dark in [false, true] {
            let k = kit(&palette(dark));
            assert_eq!(k.c.knob, Color::WHITE);
            assert_ne!(k.c.knob, k.c.layer_04);
        }
    }

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

#[cfg(test)]
mod type_and_contrast_tests {
    use super::*;
    use parley::PositionedLayoutItem;
    use rune::text::TextSystem;

    const HINDI: &str = "\u{92f}\u{939} \u{90f}\u{915} \u{92a}\u{930}\u{940}\u{915}\u{94d}\u{937}\u{923} \u{935}\u{93e}\u{915}\u{94d}\u{92f} \u{939}\u{948}";
    const TELUGU: &str = "\u{c07}\u{c26}\u{c3f} \u{c12}\u{c15} \u{c2a}\u{c30}\u{c40}\u{c15}\u{c4d}\u{c37} \u{c35}\u{c3e}\u{c15}\u{c4d}\u{c2f}\u{c02}";

    fn glyph_ids(ts: &mut TextSystem, text: &str, style: TextStyle) -> Vec<u32> {
        let layout = ts.layout_rich(text, &[], style, 1.0, None);
        let mut ids = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    ids.extend(run.glyphs().map(|g| g.id));
                }
            }
        }
        ids
    }

    fn system() -> TextSystem {
        TextSystem::new(fonts(), (SANS, "Inter"))
    }

    #[test]
    fn titles_draw_in_fraunces_and_text_in_figtree_not_in_inter() {
        let mut ts = system();
        let word = "Hamburgefonstiv 1234";
        let text = glyph_ids(&mut ts, word, TextStyle::new(14.0, 400.0));
        let inter = glyph_ids(
            &mut ts,
            word,
            TextStyle::new(14.0, 400.0).family(Family::Named("Inter")),
        );
        let title = glyph_ids(&mut ts, word, display(26.0));
        assert_ne!(text, inter, "text still falls through to Inter");
        assert_ne!(title, text, "titles still draw in the text face");
        assert!(title.iter().chain(&text).all(|id| *id != 0));
    }

    #[test]
    fn a_page_title_never_passes_the_shared_size() {
        assert_eq!(display(40.0).size, PAGE_TITLE);
        assert_eq!(display(16.0).size, 16.0);
    }

    #[test]
    fn figtree_semibold_is_a_real_weight_of_the_variable_face() {
        let mut ts = system();
        let (regular, _) = ts.measure("Meeting notes", TextStyle::new(14.0, 400.0), 1.0);
        let (bold, _) = ts.measure("Meeting notes", TextStyle::new(14.0, 600.0), 1.0);
        assert!(
            bold > regular,
            "600 measured {bold}, 400 measured {regular}"
        );
    }

    /// Hindi and Telugu transcript lines must draw letters. Linux has no system fallback
    /// here, so the bundled Noto faces carry them; Windows reads Nirmala UI.
    #[test]
    fn a_hindi_and_a_telugu_line_draw_letters_not_boxes() {
        let mut ts = system();
        for line in [HINDI, TELUGU] {
            let ids = glyph_ids(&mut ts, line, TextStyle::new(14.0, 400.0));
            assert!(!ids.is_empty());
            assert!(
                ids.iter().all(|id| *id != 0),
                "missing glyphs in {line}: {ids:?}"
            );
        }
    }

    /// The detector sees a missing glyph: a code point no font has draws the box (id 0).
    /// Windows falls back to the system's own Indic faces, so the lines above cannot be
    /// made to fail here; this proves the check is not blind.
    #[test]
    fn the_box_detector_sees_a_missing_glyph() {
        let mut ts = system();
        let ids = glyph_ids(&mut ts, "\u{10fffd}", TextStyle::new(14.0, 400.0));
        assert!(ids.contains(&0), "{ids:?}");
    }

    /// Off Windows nothing but the bundled list carries Indic text, so it must name both.
    #[test]
    fn the_non_windows_font_list_carries_both_indic_faces() {
        if cfg!(windows) {
            return;
        }
        assert_eq!(fonts().len(), bundled().len() + 2);
        assert!(SANS.contains("Noto Sans Devanagari") && SANS.contains("Noto Sans Telugu"));
    }

    fn ratio(a: Color, b: Color) -> f32 {
        let (hi, lo) = (
            a.luminance().max(b.luminance()),
            a.luminance().min(b.luminance()),
        );
        (hi + 0.05) / (lo + 0.05)
    }

    /// Every text token against every fill it is drawn on, in both modes: at least 4.5:1.
    #[test]
    fn text_tokens_keep_four_and_a_half_to_one() {
        let mut failures = Vec::new();
        for dark in [false, true] {
            let p = palette(dark);
            let k = kit(&p);
            let plain = [p.bg, p.card, p.side, p.chip, p.sel, p.hit];
            let rows: Vec<(&str, Color, Vec<Color>)> = vec![
                ("ink", p.ink, plain.to_vec()),
                ("soft", p.soft, plain.to_vec()),
                ("faint", p.faint, plain.to_vec()),
                ("accent", p.accent, vec![p.bg, p.card, p.side]),
                ("open", p.open, vec![p.bg, p.card, p.side]),
                ("danger", p.danger, vec![p.bg, p.card, p.side]),
                ("ok", p.ok, vec![p.bg, p.card, p.okbg]),
                ("warn", p.warn, vec![p.bg, p.card, p.side, p.chip, p.warnbg]),
                ("button ink", p.button_ink, vec![p.button_fill]),
                ("kit base", k.c.text_base, vec![p.card]),
                ("kit muted", k.c.text_muted, vec![p.card]),
                ("kit faint", k.c.text_faint, vec![p.card]),
                ("kit weak", k.c.text_weak, vec![p.card]),
                ("kit strong", k.c.text_strong, vec![p.card]),
            ];
            for (name, ink, fills) in rows {
                for (i, fill) in fills.into_iter().enumerate() {
                    let r = ratio(ink, fill);
                    if r < 4.5 {
                        failures.push(format!(
                            "{} {name} on fill {i}: {r:.2}",
                            if dark { "dark" } else { "light" }
                        ));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "below 4.5:1: {failures:#?}");
    }

    /// Marks people click, such as section marks, need 3:1.
    #[test]
    fn the_dimmest_text_grey_clears_three_to_one_for_clickable_marks() {
        for dark in [false, true] {
            let p = palette(dark);
            assert!(ratio(p.faint, p.bg) >= 3.0);
        }
    }

    /// Small text on a raised surface in dark mode is at least #a9a69f, and the old
    /// #77746d is not a text token any more.
    #[test]
    fn dark_small_text_on_raised_surfaces_is_at_least_a9a69f() {
        let p = palette(true);
        let floor = rune::color::hex("#a9a69f").unwrap().luminance();
        let old = rune::color::hex("#77746d").unwrap().luminance();
        for ink in [p.ink, p.soft, p.faint] {
            assert!(ink.luminance() >= floor);
            assert_ne!(ink.luminance(), old);
        }
    }
}
