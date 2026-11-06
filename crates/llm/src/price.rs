//! What a model call costs, in US dollars.
//!
//! The built-in rates come from the providers' own price pages (the reference snapshot in
//! `artifical-intelligence/documentation`, 2026-11-05). Prices change: any model can be overridden in the
//! settings (`prices`), and a model with no known price shows tokens only, never a guess.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use workcraft_core::Usage;

/// US dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    /// Rate for prompt tokens served from the cache. `None`: a tenth of `input`.
    #[serde(default)]
    pub cache_read: Option<f64>,
    /// Rate for prompt tokens written to the cache. `None`: `input` times 1.25.
    #[serde(default)]
    pub cache_write: Option<f64>,
    /// Multiplier outside the provider's peak hours (DeepSeek bills half off-peak).
    #[serde(default)]
    pub off_peak_factor: Option<f64>,
}

const fn p(input: f64, cache_write: f64, cache_read: f64, output: f64) -> Price {
    Price {
        input,
        output,
        cache_read: Some(cache_read),
        cache_write: Some(cache_write),
        off_peak_factor: None,
    }
}

const fn deepseek(miss: f64, hit: f64, output: f64) -> Price {
    Price {
        input: miss,
        output,
        cache_read: Some(hit),
        cache_write: None,
        off_peak_factor: Some(0.5),
    }
}

/// Built-in rates, matched by the start of the model name (longest first). Dots count as dashes, so
/// `claude-sonnet-5.5` and `claude-sonnet-5-5-20261001` both match.
#[must_use]
pub fn builtin(model: &str) -> Option<Price> {
    let m = model.to_ascii_lowercase().replace('.', "-");
    let leaf = m.rsplit('/').next().unwrap_or(&m);
    // (prefix, price). Longer prefixes first so `-5-5` wins over `-5`.
    const TABLE: &[(&str, Price)] = &[
        ("claude-fable-5-1", p(10.0, 12.5, 0.25, 50.0)),
        ("claude-mythos-5-1", p(10.0, 12.5, 0.25, 50.0)),
        ("claude-fable-5", p(10.0, 12.5, 1.0, 50.0)),
        ("claude-opus-5-5", p(4.0, 5.0, 0.20, 20.0)),
        ("claude-opus-5", p(5.0, 6.25, 0.50, 25.0)),
        ("claude-opus-4-8", p(5.0, 6.25, 0.50, 25.0)),
        ("claude-opus-4-7", p(5.0, 6.25, 0.50, 25.0)),
        ("claude-opus-4-6", p(5.0, 6.25, 0.50, 25.0)),
        ("claude-opus-4-5", p(5.0, 6.25, 0.50, 25.0)),
        ("claude-sonnet-5-5", p(2.0, 2.5, 0.10, 10.0)),
        ("claude-sonnet-5", p(2.0, 2.5, 0.20, 10.0)),
        ("claude-sonnet-4", p(3.0, 3.75, 0.30, 15.0)),
        ("claude-haiku-5-5", p(0.10, 0.125, 0.01, 0.50)),
        ("claude-haiku-4-5", p(1.0, 1.25, 0.10, 5.0)),
        ("deepseek-v4-pro", deepseek(1.32, 0.044, 3.96)),
        // The legacy names are served by the Flash model and billed at the Flash price.
        ("deepseek-flash", deepseek(0.3, 0.006, 1.2)),
        ("deepseek-v4-flash", deepseek(0.3, 0.006, 1.2)),
        ("deepseek-chat", deepseek(0.3, 0.006, 1.2)),
        ("deepseek-reasoner", deepseek(0.3, 0.006, 1.2)),
    ];
    TABLE
        .iter()
        .find(|(prefix, _)| leaf.starts_with(prefix))
        .map(|(_, price)| *price)
}

/// DeepSeek peak hours: 01:00 to 04:00 and 06:00 to 10:00 UTC, Monday to Friday. Chinese public holidays are
/// off-peak at DeepSeek but are not known here, so they are counted as peak (the estimate errs high).
#[must_use]
pub fn is_peak(now_secs: u64) -> bool {
    let days = now_secs / 86_400;
    // 1970-01-01 was a Thursday; with Sunday = 0 that is 4.
    let weekday = (days + 4) % 7;
    let hour = (now_secs % 86_400) / 3600;
    (1..=5).contains(&weekday) && ((1..4).contains(&hour) || (6..10).contains(&hour))
}

/// Cost of `usage`, or `None` when the model has no price. `local` models cost nothing.
#[must_use]
pub fn cost_usd(
    provider: &str,
    model: &str,
    usage: &Usage,
    overrides: &BTreeMap<String, Price>,
    local: bool,
    now_secs: u64,
) -> Option<f64> {
    if local {
        return Some(0.0);
    }
    let price = overrides
        .get(&format!("{provider}:{model}"))
        .or_else(|| overrides.get(model))
        .copied()
        .or_else(|| builtin(model))?;
    let factor = match price.off_peak_factor {
        Some(f) if !is_peak(now_secs) => f,
        _ => 1.0,
    };
    let per = |tokens: u64, rate: f64| tokens as f64 * rate * factor / 1_000_000.0;
    Some(
        per(usage.input_tokens, price.input)
            + per(usage.output_tokens, price.output)
            + per(
                usage.cache_read_tokens,
                price.cache_read.unwrap_or(price.input * 0.1),
            )
            + per(
                usage.cache_write_tokens,
                price.cache_write.unwrap_or(price.input * 1.25),
            ),
    )
}

pub use workcraft_core::format_usd;

#[cfg(test)]
mod tests {
    use super::*;

    // Wednesday 2026-11-04 08:00 UTC (peak) and 12:00 UTC (off-peak), Saturday 2026-11-07 08:00 UTC (off-peak).
    const WED_08: u64 = 1_793_779_200;
    const WED_12: u64 = 1_793_793_600;
    const SAT_08: u64 = 1_794_038_400;

    #[test]
    fn peak_hours() {
        assert!(is_peak(WED_08));
        assert!(!is_peak(WED_12));
        assert!(!is_peak(SAT_08));
    }

    #[test]
    fn deepseek_cost_halves_off_peak_and_cached_input_is_cheap() {
        let u = Usage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
            ..Usage::default()
        };
        let none = BTreeMap::new();
        let peak = cost_usd("deepseek", "deepseek-flash", &u, &none, false, WED_08).unwrap();
        assert!((peak - (0.3 + 1.2 + 0.006)).abs() < 1e-9, "{peak}");
        let off = cost_usd("deepseek", "deepseek-flash", &u, &none, false, WED_12).unwrap();
        assert!((off - peak / 2.0).abs() < 1e-9);
    }

    #[test]
    fn claude_cost_uses_cache_rates_and_matches_dated_names() {
        let u = Usage {
            input_tokens: 1_000_000,
            output_tokens: 100_000,
            cache_read_tokens: 2_000_000,
            cache_write_tokens: 1_000_000,
            ..Usage::default()
        };
        let c = cost_usd(
            "anthropic",
            "claude-sonnet-5-5-20261001",
            &u,
            &BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        assert!((c - (2.0 + 1.0 + 0.2 + 2.5)).abs() < 1e-9, "{c}");
        // `-5-5` is not mistaken for plain `-5`.
        assert_eq!(builtin("claude-opus-5.5").unwrap().input, 4.0);
        assert_eq!(builtin("claude-opus-5").unwrap().input, 5.0);
    }

    #[test]
    fn unknown_models_have_no_price_and_local_ones_cost_nothing() {
        let u = Usage {
            input_tokens: 5,
            ..Usage::default()
        };
        assert_eq!(
            cost_usd("x", "mystery", &u, &BTreeMap::new(), false, 0),
            None
        );
        assert_eq!(
            cost_usd("ollama", "qwen3.5:4b", &u, &BTreeMap::new(), true, 0),
            Some(0.0)
        );
        let mut o = BTreeMap::new();
        o.insert(
            "x:mystery".to_string(),
            Price {
                input: 1.0,
                output: 2.0,
                cache_read: None,
                cache_write: None,
                off_peak_factor: None,
            },
        );
        let big = Usage {
            input_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
            ..Usage::default()
        };
        assert_eq!(
            cost_usd("x", "mystery", &big, &o, false, 0),
            Some(1.0 + 0.1)
        );
    }

    #[test]
    fn dollars_read_short() {
        assert_eq!(format_usd(0.0), "$0");
        assert_eq!(format_usd(0.00004), "< $0.0001");
        assert_eq!(format_usd(0.0123), "$0.0123");
        assert_eq!(format_usd(3.456), "$3.46");
    }
}
