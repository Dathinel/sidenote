//! Text roles: the few sizes, weights and greys the app reads in. See docs/design/typography.md.
//! Pages use these instead of raw sizes, so every page keeps the same hierarchy.
use super::theme::{BODY, BODY_LH, Palette};
use rune::{Color, El, TextStyle, row, text};

/// A size, weight and line height. The colour comes from the caller's palette.
#[derive(Clone, Copy)]
pub struct Role {
    pub size: f32,
    pub weight: f32,
    pub lh: f32,
}

impl Role {
    /// The same size in the title weight, for a name inside a dense row or a lead word.
    pub const fn bold(self) -> Role {
        Role {
            weight: 600.0,
            ..self
        }
    }

    /// The same size at 500, for controls and warnings that need a little weight.
    pub const fn medium(self) -> Role {
        Role {
            weight: 500.0,
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

/// Page sections: Decisions, To do, Needs attention, a settings group.
pub const SECTION: Role = Role {
    size: 20.0,
    weight: 600.0,
    lh: 28.0,
};
/// Item titles: a meeting in a list, a topic, a model card, a panel's subject.
pub const TITLE: Role = Role {
    size: 16.0,
    weight: 600.0,
    lh: 24.0,
};
/// Reading text: summary points, to-dos, answers and live notes.
pub const READ: Role = Role {
    size: BODY,
    weight: 400.0,
    lh: BODY_LH,
};
/// Dense rows and descriptions: tasks, findings, transcript lines, form text.
pub const SMALL: Role = Role {
    size: 14.0,
    weight: 400.0,
    lh: 21.0,
};
/// The name in a dense row: a setting, a finding, a memory claim, a sidebar place.
pub const NAME: Role = SMALL.bold();
/// Tabs, buttons, links and menu items.
pub const CONTROL: Role = Role {
    size: 14.0,
    weight: 500.0,
    lh: 20.0,
};
/// Facts about an item: times, people, dates, sizes, licences, sources and help text.
pub const META: Role = Role {
    size: 12.0,
    weight: 400.0,
    lh: 18.0,
};
/// Group labels inside a section (Today, This week, Medium) and field labels.
pub const GROUP: Role = Role {
    size: 13.0,
    weight: 600.0,
    lh: 20.0,
};
/// Panel captions in capitals, and the number in a count pill.
pub const CAPTION: Role = Role {
    size: 12.0,
    weight: 600.0,
    lh: 16.0,
};

/// Space above a section heading. Sections are grouped by space first, lines second.
pub const SECTION_TOP: f32 = 40.0;
/// Space between a section heading and its first item.
pub const SECTION_GAP: f32 = 12.0;
/// Space above a group label inside a section.
pub const GROUP_TOP: f32 = 20.0;
/// Space between a title and its meta line.
pub const META_GAP: f32 = 2.0;

/// The " · " that joins facts on one meta line.
pub const DOT: &str = "  \u{b7}  ";

/// Meta text in the scanning grey.
pub fn meta(s: impl Into<String>, p: Palette) -> El {
    META.wrap(s, p.faint)
}

/// Several facts on one quiet line, joined by " · ". Empty parts are left out. Each fact keeps
/// its words together, so a narrow window breaks the line only between facts.
pub fn meta_line<S: AsRef<str>>(parts: &[S], p: Palette) -> El {
    let joined = parts
        .iter()
        .map(AsRef::as_ref)
        .filter(|s| !s.is_empty())
        .map(|s| s.replace(' ', "\u{a0}"))
        .collect::<Vec<_>>()
        .join(DOT);
    META.wrap(joined, p.faint)
}

/// A section heading: the role, the shared spacing and an optional count pill.
pub fn section(s: impl Into<String>, count_: Option<usize>, p: Palette) -> El {
    let mut r = row()
        .items_center()
        .gap(8.0)
        .mt(SECTION_TOP)
        .mb(SECTION_GAP)
        .child(SECTION.wrap(s, p.ink));
    if let Some(n) = count_ {
        r = r.child(count(n, p));
    }
    r
}

/// A number after a heading or tab, in a small pill. `soft` keeps 4.5:1 on the `chip` fill.
pub fn count(n: usize, p: Palette) -> El {
    text(n.to_string())
        .font(META.medium().style())
        .color(p.soft)
        .px(7.0)
        .radius(999.0)
        .fill(p.chip)
}
