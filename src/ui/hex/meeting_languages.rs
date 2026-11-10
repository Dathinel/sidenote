use super::*;

pub(super) fn section(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let options = [
        "Existing pipeline (default)",
        "Automatic (experimental)",
        "Off: first declared language",
        "Always English",
        "Always Hindi",
        "Always mixed",
    ]
    .map(str::to_owned);
    let config = &s.config.pipeline;
    let current = if !config.enabled() {
        0
    } else {
        match config.language_mode.as_str() {
            "off" => 2,
            "forced:en" => 3,
            "forced:hi" => 4,
            "forced:mixed" => 5,
            _ => 1,
        }
    };
    let control = Select::new("meeting-language-mode", &options)
        .label("Language handling")
        .current(Some(options[current].as_str()))
        .appearance(SelectAppearance::Field)
        .width(280.0)
        .open(s.hex.select_open.as_deref() == Some("meeting-language-mode"))
        .on_toggle(|s: &mut State| {
            s.hex.select_open = if s.hex.select_open.is_some() {
                None
            } else {
                Some("meeting-language-mode".into())
            }
        })
        .on_close(|s: &mut State| s.hex.select_open = None)
        .on_choose(|s: &mut State, value: &str, _: &mut Tx| {
            let result = sidenote::core::Config::load_in(&s.store.root).and_then(|mut config| {
                config.pipeline.profile = if value == "Existing pipeline (default)" {
                    "compat"
                } else {
                    "experimental"
                }
                .into();
                config.pipeline.language_mode = match value {
                    "Off: first declared language" => "off",
                    "Always English" => "forced:en",
                    "Always Hindi" => "forced:hi",
                    "Always mixed" => "forced:mixed",
                    _ => "auto",
                }
                .into();
                config.save(&s.store.root)?;
                Ok(config)
            });
            match result {
                Ok(config) => {
                    s.config = config;
                    s.feature_error = None;
                    s.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
                }
                Err(_) => {
                    s.feature_error = Some("Could not save language handling. Try again.".into())
                }
            }
            s.hex.select_open = None;
        })
        .build(k, layer);
    let mut body=col().w_full().gap(6.0).py(10.0)
        .child(roles::NAME.wrap("Language handling", p.ink)).child(control)
        .child(roles::META.wrap("Experimental: English and Hindi only. English only adds no detector or Hindi model. Always mixed uses the mixed Hindi-English model for every part.", p.soft).w_full().min_w(0.0));
    if !cfg!(feature = "asr-runtime") {
        body = body.child(roles::META.line(
            "This build keeps the English reading when local inference is unavailable.",
            p.soft,
        ));
    }
    if let Some(error) = &s.feature_error {
        body = body.child(roles::SMALL.medium().line(error, p.warn).role("alert"));
    }
    body.key("meeting-language-mode")
}
