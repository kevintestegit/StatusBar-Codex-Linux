#[derive(Clone, Copy)]
pub struct Price {
    pub input: f64,
    pub cached_input: f64,
    pub output: f64,
}

pub fn price(model: &str) -> Option<Price> {
    match normalize_model(model).as_str() {
        "gpt-5.5" => Some(Price {
            input: 5.00,
            cached_input: 0.50,
            output: 30.00,
        }),
        "gpt-5.4" => Some(Price {
            input: 2.50,
            cached_input: 0.25,
            output: 15.00,
        }),
        "gpt-5.4-mini" => Some(Price {
            input: 0.75,
            cached_input: 0.075,
            output: 4.50,
        }),
        "gpt-5.4-nano" => Some(Price {
            input: 0.20,
            cached_input: 0.02,
            output: 1.25,
        }),
        "gpt-5.2" | "gpt-5.2-codex" => Some(Price {
            input: 1.75,
            cached_input: 0.175,
            output: 14.00,
        }),
        "gpt-5.1" | "gpt-5.1-codex" | "gpt-5.1-codex-max" | "gpt-5" | "gpt-5-codex" => {
            Some(Price {
                input: 1.25,
                cached_input: 0.125,
                output: 10.00,
            })
        }
        "gpt-5-mini" => Some(Price {
            input: 0.25,
            cached_input: 0.025,
            output: 2.00,
        }),
        "gpt-5-nano" => Some(Price {
            input: 0.05,
            cached_input: 0.005,
            output: 0.40,
        }),
        _ => None,
    }
}

pub fn normalize_model(model: &str) -> String {
    if model.starts_with("gpt-5.5") {
        "gpt-5.5".into()
    } else if model.starts_with("gpt-5.4-mini") {
        "gpt-5.4-mini".into()
    } else if model.starts_with("gpt-5.4-nano") {
        "gpt-5.4-nano".into()
    } else if model.starts_with("gpt-5.4") {
        "gpt-5.4".into()
    } else if model.starts_with("gpt-5.2-codex") {
        "gpt-5.2-codex".into()
    } else if model.starts_with("gpt-5.2") {
        "gpt-5.2".into()
    } else if model.starts_with("gpt-5.1-codex-max") {
        "gpt-5.1-codex-max".into()
    } else if model.starts_with("gpt-5.1-codex") {
        "gpt-5.1-codex".into()
    } else if model.starts_with("gpt-5.1") {
        "gpt-5.1".into()
    } else if model.starts_with("gpt-5-mini") {
        "gpt-5-mini".into()
    } else if model.starts_with("gpt-5-nano") {
        "gpt-5-nano".into()
    } else if model.starts_with("gpt-5-codex") {
        "gpt-5-codex".into()
    } else if model.starts_with("gpt-5") {
        "gpt-5".into()
    } else if model.is_empty() {
        "unknown".into()
    } else {
        model.into()
    }
}
