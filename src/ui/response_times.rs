//! Uses the same settings cards and controls as the surrounding Diagnostics page.
use super::{
    State,
    hex::settings::card,
    roles,
    theme::Palette,
    widgets::{ButtonRole, button},
};
use rune::{El, col, row};
use sidenote::response_time;

pub fn section(p: Palette) -> El {
    let enabled = response_time::enabled();
    let controls = row()
        .gap(8.0)
        .child(
            button(
                if enabled { "Disable" } else { "Enable" },
                "",
                ButtonRole::Primary,
                p,
            )
            .key("response-times-toggle")
            .on_press(|_: &mut State| response_time::set_enabled(!response_time::enabled())),
        )
        .child(
            button("Clear", "", ButtonRole::Secondary, p)
                .key("response-times-clear")
                .on_press(|_: &mut State| {
                    let enabled = response_time::enabled();
                    response_time::set_enabled(false);
                    response_time::set_enabled(enabled);
                }),
        )
        .child(
            button("Refresh", "", ButtonRole::Secondary, p)
                .key("response-times-refresh")
                .on_press(|_: &mut State| {}),
        );
    let mut body = col().w_full().gap(12.0).padding(14.0)
        .child(roles::SMALL.wrap(
            "Response times are off by default and kept only in memory. Turning recording off clears samples. No meeting content or persistent identifiers are recorded.", p.soft))
        .child(controls)
        .child(roles::SMALL.wrap(
            "Encoded-frame proxy: first drawn feedback / first drawn ready state. This does not measure physical display latency. Percentiles use the last 256 successful samples per action; counts cover this recording session.", p.soft));
    if let Some((summary, drops)) = response_time::snapshot() {
        let unmatched: u64 = summary.iter().map(|sample| sample.unmatched).sum();
        body = body.child(roles::META.wrap(format!("Recording on · dropped records: {drops} · unmatched: {unmatched}. Refresh to read the latest worker snapshot."), p.soft));
        let mut any = false;
        for (name, sample) in response_time::ACTIONS.iter().zip(summary) {
            if sample.count + sample.failed + sample.superseded == 0 {
                continue;
            }
            any = true;
            body = body.child(
                col()
                    .w_full()
                    .gap(4.0)
                    .child(roles::SMALL.wrap(*name, p.ink))
                    .child(roles::META.wrap(
                        format!(
                            "{} completed · {} failed · {} superseded",
                            sample.count, sample.failed, sample.superseded
                        ),
                        p.soft,
                    ))
                    .children((sample.count > 0).then(|| {
                        roles::SMALL.wrap(
                            format!(
                                "First: p50 {:.1} / p95 {:.1} ms. Ready: p50 {:.1} / p95 {:.1} ms.",
                                sample.first_p50_ms,
                                sample.first_p95_ms,
                                sample.complete_p50_ms,
                                sample.complete_p95_ms
                            ),
                            p.ink,
                        )
                    })),
            );
        }
        if !any {
            body = body.child(roles::SMALL.wrap(
                "No completed samples yet. Use an app action, then return here and Refresh.",
                p.soft,
            ));
        }
    } else {
        body = body.child(roles::SMALL.wrap(
            if enabled {
                "Snapshot busy. Refresh again."
            } else {
                "Recording off. Enable it to measure actions in this session."
            },
            p.soft,
        ));
    }
    card("Response times", body, p).key("response-times")
}
