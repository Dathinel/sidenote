use super::{
    Capture, El, Palette, State, col, el, frame_image, pressable, roles, row, step_details, theme,
    time,
};
use rune::{Key, KeyEvent, Mods, SurfaceFit, Tx};
use rune_kit::{ScrollBar, ScrollBarStyle, SegmentedControl};
use sidenote::core::UiPrefs;

const STRIP: &str = "annotation-filmstrip";
const THUMB_WIDTH: f32 = 176.0;
const THUMB_HEIGHT: f32 = 99.0;
const THUMB_RADIUS: f32 = 6.0;

pub(super) fn view_switch(s: &State, p: Palette) -> El {
    let mut kit = theme::kit(&p);
    kit.c.bg_base = p.sel;
    kit.c.border_strong = p.ink;
    SegmentedControl::new(
        &["Blocks", "Filmstrip"],
        (!s.annotations.transcript).then_some(usize::from(s.annotations.filmstrip)),
    )
    .label("Annotation view")
    .width(Some(148.0))
    .on_pick::<State>(|s, index, tx| {
        let filmstrip = index == 1;
        if filmstrip != s.annotations.filmstrip {
            s.annotations.filmstrip = filmstrip;
            let mut prefs = UiPrefs::load(&s.store.root);
            prefs.annotations_view = index;
            if prefs.save(&s.store.root).is_err() {
                s.annotations
                    .errors
                    .push("Could not save the annotation view.".into());
            }
        }
        s.annotations.transcript = false;
        tx.scroll_to_y("annotations-pane", 0.0);
    })
    .build(&kit)
    .key("annotation-view")
}

pub(super) fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    let a = &s.annotations;
    if !a.filmstrip
        || a.transcript
        || a.menu_open
        || a.busy
        || a.note_open
        || a.review.is_some()
        || a.notebooks.selected.is_some()
        || a.notebooks.shared.is_some()
        || a.notebooks.picker.is_some()
        || e.mods != Mods::default()
    {
        return false;
    }
    let delta = match e.key {
        Key::Left => -1,
        Key::Right => 1,
        _ => return false,
    };
    let Some(last) = a
        .captures
        .iter()
        .find(|c| Some(&c.folder) == a.selected.as_ref())
        .and_then(|capture| capture.data.keyframes.len().checked_sub(1))
    else {
        return false;
    };
    let next = a
        .filmstrip_step
        .min(last)
        .saturating_add_signed(delta)
        .min(last);
    select_step(s, next, tx);
    true
}

fn thumb_key(index: usize) -> String {
    format!("annotation-thumbnail-{}", index + 1)
}

fn select_step(s: &mut State, index: usize, tx: &mut Tx) {
    s.annotations.filmstrip_step = index;
    tx.scroll_into_view_in(STRIP, thumb_key(index));
}

pub(super) fn view(s: &State, c: &Capture, width: f32, p: Palette) -> El {
    let index = s.annotations.filmstrip_step.min(c.data.keyframes.len() - 1);
    let frame = &c.data.keyframes[index];
    let mut thumbs = row().gap(10.0).padding(4.0);
    for (i, frame) in c.data.keyframes.iter().enumerate() {
        let selected = index == i;
        let mut shot = el()
            .size(THUMB_WIDTH, THUMB_HEIGHT)
            .clip_rounded(THUMB_RADIUS)
            .fill(p.card);
        if let Some(Ok(image)) = s.annotations.images.get(i) {
            shot = shot.pixels_fit(image.pixels(), SurfaceFit::Fit);
        }
        // Paint the border over the image, outside the image's rounded clip.
        let thumbnail = col().child(shot).child(
            el().absolute(0.0, 0.0)
                .size(THUMB_WIDTH, THUMB_HEIGHT)
                .radius(THUMB_RADIUS)
                .border(
                    if selected { 2.0 } else { 1.0 },
                    if selected { p.ink } else { p.line },
                ),
        );
        let number = i + 1;
        let timestamp = time(frame.t);
        thumbs = thumbs.child(
            pressable(col().w(THUMB_WIDTH).gap(6.0))
                .key(thumb_key(i))
                .label(format!("Step {number}, {timestamp}"))
                .checked(selected)
                .focus_ring(2.0, 2.0, p.ink)
                .child(thumbnail)
                .child(roles::META.line(
                    format!("{number}{}{timestamp}", roles::DOT),
                    if selected { p.ink } else { p.faint },
                ))
                .on_press_tx(move |s: &mut State, tx| select_step(s, i, tx)),
        );
    }
    col()
        .w_full()
        .gap(14.0)
        .key("annotation-filmstrip-view")
        .child(frame_image(s, c, index, frame, width.min(680.0), p))
        .child(step_details(c, index, frame, p, true).w_full().max_w(680.0))
        .child(
            col()
                .key(STRIP)
                .w_full()
                .min_w(0.0)
                .scroll_x()
                .child(thumbs)
                .scroll_bar_styled(ScrollBarStyle::native(p.soft, p.ink).horizontal()),
        )
}
