//! Colours, fonts and the type scale. Light and dark follow the system setting. `faint` keeps
//! 4.5:1 on the page and on raised surfaces in both modes.
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
    pub chip: Color,
    pub hit: Color,
    pub shadow: Color,
}

pub fn palette(dark: bool) -> Palette {
    let c = if dark {
        [
            "#1a1a18",
            "#242422",
            "#141413",
            "#e9e7e1",
            "#b0ada4",
            "#8f8c83",
            "#302f2c",
            "#8db5dc",
            "#69c995",
            "#1b2c22",
            "#e3aa55",
            "#33281a",
            "#b49ee3",
            "#2a2a27",
            "#3a3317",
            "#00000066",
        ]
    } else {
        [
            "#fcfbf8",
            "#ffffff",
            "#f3f2ed",
            "#1e1e1c",
            "#5d5b55",
            "#6e6b64",
            "#e6e3db",
            "#2f5d8a",
            "#1d7a4c",
            "#e4f3eb",
            "#a35d00",
            "#fcf0dc",
            "#6a4ea3",
            "#efede7",
            "#fff3c4",
            "#1e1e1c1f",
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
        hit: c[14],
        shadow: c[15],
    }
}

/// Kit controls (text fields, the runner dropdown, Markdown) drawn from the same palette.
pub fn kit(p: &Palette) -> Kit {
    Kit::from_palette(PaletteSeed {
        bg: p.card,
        fg: p.ink,
        accent: p.accent,
    })
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
