//! Text roles: the few sizes, weights and greys the app reads in. See docs/design/typography.md.
//! Prefer these over raw sizes so pages keep the same hierarchy.
use super::theme::{BODY, BODY_LH, Palette};
use rune::{Color, El, TextStyle, text};

/// A size, weight and line height. The colour comes from the caller's palette.
#[derive(Clone, Copy)]
pub struct Role {
    pub size: f32,
    pub weight: f32,
    pub lh: f32,
}

impl Role {
    /// The same size in the title weight, for a lead word or a name inside a line.
    pub const fn bold(self) -> Role {
        Role {
            weight: 600.0,
            ..self
        }
    }

    pub fn style(self) -> TextStyle {
        TextStyle::new(self.size, self.weight).lh(self.lh)
    }

    /// Wrapping text in this role.
    pub fn wrap(self, s: impl Into<String>, ink: Color) -> El {
        text(s).font(self.style()).color(ink).wrap()
    }

    /// One line that ends in an ellipsis when it does not fit.
    pub fn line(self, s: impl Into<String>, ink: Color) -> El {
        text(s).font(self.style()).color(ink).truncate().min_w(0.0)
    }
}

/// Decisions, To do, settings groups.
pub const SECTION: Role = Role {
    size: 18.0,
    weight: 600.0,
    lh: 26.0,
};
/// List row titles, topic titles and card titles.
pub const TITLE: Role = Role {
    size: 16.0,
    weight: 600.0,
    lh: 24.0,
};
/// Reading text: summary points, to-dos and answers.
pub const READ: Role = Role {
    size: BODY,
    weight: 400.0,
    lh: BODY_LH,
};
/// Transcript lines, form text and descriptions.
pub const SMALL: Role = Role {
    size: 14.0,
    weight: 400.0,
    lh: 21.0,
};
/// Tabs, buttons and menu items.
pub const CONTROL: Role = Role {
    size: 14.0,
    weight: 500.0,
    lh: 21.0,
};
/// Times, people, dates, sources and help text.
pub const META: Role = Role {
    size: 13.0,
    weight: 400.0,
    lh: 20.0,
};
/// Day groups and field labels.
pub const GROUP: Role = Role {
    size: 13.0,
    weight: 600.0,
    lh: 20.0,
};
/// Panel captions in capitals, and the number in a count pill.
pub const CAPTION: Role = Role {
    size: 12.0,
    weight: 600.0,
    lh: 18.0,
};

/// Meta text in the scanning grey.
pub fn meta(s: impl Into<String>, p: Palette) -> El {
    META.wrap(s, p.faint)
}

/// A number after a heading or tab, in a small pill. `soft` because `faint` loses contrast on
/// the dark `chip` fill.
pub fn count(n: usize, p: Palette) -> El {
    text(n.to_string())
        .font(TextStyle::new(CAPTION.size, 500.0).lh(CAPTION.lh))
        .color(p.soft)
        .px(7.0)
        .radius(999.0)
        .fill(p.chip)
}
