//! "Should I /clear?" advisor.
//!
//! Decides whether a session has reached a point where running `/clear` in
//! Claude Code would help — and explains *why*, so the user learns to feel the
//! boundary themselves. Used both live (in `watch`) and post-hoc (in `diagnose`).
//!
//! The signal combines three things AgentSight already measures:
//!   1. Cache stability — churning/degrading means context is no longer earning
//!      its keep (it's being rebuilt each turn instead of cheaply reused).
//!   2. Context growth — input ballooning since turn 5.
//!   3. Live context *fill* — how full the context window is right now, measured
//!      as carried tokens on the most recent turn divided by the model's window.
//!
//! Fill is a fraction, not an absolute, so the advice is right whether you're on
//! a 200k-window model or a 1M one. None of these alone justifies a clear —
//! `/clear` helps most when context is *both* full *and* not pulling its weight.

use crate::parser::types::TurnSummary;

use super::cache::analyze_cache_stability;
use super::context::analyze_context_growth;
use super::types::CacheClassification;

/// Default context window when the model doesn't advertise an extended one.
pub const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;
/// Extended ("[1m]") context window.
pub const EXTENDED_CONTEXT_WINDOW: u64 = 1_000_000;

/// Window fill above which context is "large" — half the window in one turn.
const FILL_LARGE: f64 = 0.50;
/// Window fill above which context is "very large" — the window is filling up
/// and latency/quality start to suffer regardless of cache health.
const FILL_HUGE: f64 = 0.80;

/// Sessions shorter than this are never flagged — too little signal, and the
/// cost of re-establishing context outweighs any benefit.
const MIN_TURNS: usize = 5;

/// Infer the context window from a model id. Models tagged `[1m]` (or otherwise
/// carrying a `1m` marker) get the extended window; everything else the default.
///
/// This is only a *hint* — the model string often omits the `[1m]` tag even when
/// an extended window is in use. [`advise_clear`] reconciles it against observed
/// usage so the reported fill never exceeds 100% spuriously.
pub fn detect_context_window(model: Option<&str>) -> u64 {
    match model {
        Some(m) if m.to_ascii_lowercase().contains("1m") => EXTENDED_CONTEXT_WINDOW,
        _ => DEFAULT_CONTEXT_WINDOW,
    }
}

/// Reconcile the model hint with observed usage: the true window can't be smaller
/// than the most context we ever carried, so promote the hint to the smallest
/// tier that actually fits `peak` (and beyond known tiers, use `peak` itself).
fn effective_window(hint: u64, peak: u64) -> u64 {
    if peak <= hint {
        hint
    } else if peak <= EXTENDED_CONTEXT_WINDOW {
        EXTENDED_CONTEXT_WINDOW
    } else {
        peak
    }
}

/// How strongly AgentSight suggests running `/clear`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClearUrgency {
    /// Context is healthy — keep going.
    Healthy,
    /// Some signal present — clear at your next natural task boundary.
    Consider,
    /// Strong signal — context is large and not earning its keep.
    Recommend,
}

impl ClearUrgency {
    /// Short uppercase label for table/column display.
    pub fn label(&self) -> &'static str {
        match self {
            ClearUrgency::Healthy => "ok",
            ClearUrgency::Consider => "consider",
            ClearUrgency::Recommend => "CLEAR",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ClearUrgency::Healthy => "healthy",
            ClearUrgency::Consider => "consider",
            ClearUrgency::Recommend => "recommend",
        }
    }
}

/// The advisor's verdict for one session.
#[derive(Debug, Clone)]
pub struct ClearAdvice {
    pub urgency: ClearUrgency,
    /// Tokens carried on the most recent turn (≈ current context-window fill).
    pub current_context_tokens: u64,
    /// Largest per-turn carried context seen in the session.
    pub peak_context_tokens: u64,
    /// The context window this verdict was judged against.
    pub context_window: u64,
    /// Fraction of the window currently filled (0.0–1.0+).
    pub context_fraction: f64,
    /// 1-based turn where cache churn first persisted past warm-up, if any.
    pub churn_onset_turn: Option<usize>,
    pub classification: CacheClassification,
    pub growth_flagged: bool,
    pub growth_factor: f64,
    /// Human-readable signals that drove the urgency, most important first.
    pub reasons: Vec<String>,
}

impl ClearAdvice {
    /// A short, friendly one-line suggestion suitable for printing.
    pub fn headline(&self) -> String {
        match self.urgency {
            ClearUrgency::Healthy => "Context looks healthy — no need to /clear yet.".to_string(),
            ClearUrgency::Consider => "Consider /clear at your next task boundary.".to_string(),
            ClearUrgency::Recommend => {
                "Recommend /clear before starting the next task.".to_string()
            }
        }
    }
}

/// Project a [`ClearAdvice`] into its JSON representation. Shared by `diagnose`
/// (post-hoc) and `watch` (live NDJSON) so both emit the same shape.
pub fn clear_advice_to_json(advice: &ClearAdvice) -> crate::output::json::ClearAdviceJson {
    crate::output::json::ClearAdviceJson {
        urgency: advice.urgency.as_str().to_string(),
        current_context_tokens: advice.current_context_tokens,
        peak_context_tokens: advice.peak_context_tokens,
        context_window: advice.context_window,
        context_fraction: advice.context_fraction,
        churn_onset_turn: advice.churn_onset_turn,
        reasons: advice.reasons.clone(),
    }
}

/// Carried context for a turn ≈ everything the model had to read: fresh input
/// plus cache writes plus cache reads.
fn carried_context(turn: &TurnSummary) -> u64 {
    turn.usage.input_tokens
        + turn.usage.cache_creation_input_tokens
        + turn.usage.cache_read_input_tokens
}

/// First turn (1-based) past warm-up whose cache-creation ratio stays above 30%.
/// The first few turns naturally front-load the cache, so we skip them.
fn detect_churn_onset(per_turn_ratios: &[f64]) -> Option<usize> {
    per_turn_ratios
        .iter()
        .enumerate()
        .skip(MIN_TURNS)
        .find(|(_, r)| **r > 0.30)
        .map(|(i, _)| i + 1)
}

/// Judge whether this session should be `/clear`ed, and explain why.
///
/// `context_window` is the model's token window — use [`detect_context_window`]
/// to derive it from the session's model id.
pub fn advise_clear(turns: &[TurnSummary], context_window: u64) -> ClearAdvice {
    let current_context_tokens = turns.last().map(carried_context).unwrap_or(0);
    let peak_context_tokens = turns.iter().map(carried_context).max().unwrap_or(0);
    let window = effective_window(context_window.max(1), peak_context_tokens);
    let context_fraction = current_context_tokens as f64 / window as f64;

    if turns.len() < MIN_TURNS {
        return ClearAdvice {
            urgency: ClearUrgency::Healthy,
            current_context_tokens,
            peak_context_tokens,
            context_window: window,
            context_fraction,
            churn_onset_turn: None,
            classification: CacheClassification::Stable,
            growth_flagged: false,
            growth_factor: 0.0,
            reasons: Vec::new(),
        };
    }

    let cache = analyze_cache_stability(turns);
    let growth = analyze_context_growth(turns);
    let churn_onset_turn = detect_churn_onset(&cache.per_turn_ratios);

    let mut score: u32 = 0;
    let mut reasons: Vec<String> = Vec::new();

    match cache.classification {
        CacheClassification::Churning => {
            score += 1;
            let where_ = churn_onset_turn
                .map(|t| format!(" since turn {t}"))
                .unwrap_or_default();
            reasons.push(format!(
                "cache is churning{where_} — context is being rebuilt each turn instead of reused"
            ));
        }
        CacheClassification::Degrading => {
            score += 2;
            reasons.push(
                "cache efficiency is degrading as the session grows — a fresh context would reset it"
                    .to_string(),
            );
        }
        CacheClassification::Stable => {}
    }

    if growth.flagged {
        score += 1;
        reasons.push(format!(
            "context has grown {:.1}x since turn 5",
            growth.growth_factor
        ));
    }

    if context_fraction >= FILL_HUGE {
        score += 2;
        reasons.push(format!(
            "context window is {:.0}% full — latency and quality suffer",
            context_fraction * 100.0
        ));
    } else if context_fraction >= FILL_LARGE {
        score += 1;
        reasons.push(format!(
            "context window is {:.0}% full — a lot to re-read every turn",
            context_fraction * 100.0
        ));
    }

    let urgency = if score >= 3 {
        ClearUrgency::Recommend
    } else if score >= 1 {
        ClearUrgency::Consider
    } else {
        ClearUrgency::Healthy
    };

    ClearAdvice {
        urgency,
        current_context_tokens,
        peak_context_tokens,
        context_window: window,
        context_fraction,
        churn_onset_turn,
        classification: cache.classification,
        growth_flagged: growth.flagged,
        growth_factor: growth.growth_factor,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::diagnose::test_helpers::make_turn;

    const W: u64 = DEFAULT_CONTEXT_WINDOW;

    #[test]
    fn short_session_is_always_healthy() {
        let turns: Vec<_> = (0..3)
            .map(|i| make_turn(i, 1000, 5000, 200_000, 100, vec![]))
            .collect();
        let advice = advise_clear(&turns, W);
        assert_eq!(advice.urgency, ClearUrgency::Healthy);
        assert!(advice.reasons.is_empty());
    }

    #[test]
    fn small_stable_session_is_healthy() {
        // Low cache creation, small carried context, flat growth -> healthy.
        let turns: Vec<_> = (0..10)
            .map(|i| make_turn(i, 100, 50, 5_000, 100, vec![]))
            .collect();
        let advice = advise_clear(&turns, W);
        assert_eq!(advice.urgency, ClearUrgency::Healthy);
    }

    #[test]
    fn churning_plus_full_window_recommends_clear() {
        // Sustained high cache creation (churning) AND a window that's >80% full.
        let turns: Vec<_> = (0..12)
            .map(|i| make_turn(i, 100, 90_000, 100_000, 100, vec![]))
            .collect();
        let advice = advise_clear(&turns, W); // ~190k carried / 200k = 95% full
        assert_eq!(advice.urgency, ClearUrgency::Recommend);
        assert_eq!(advice.classification, CacheClassification::Churning);
        assert!(advice.context_fraction >= FILL_HUGE);
        assert!(advice.churn_onset_turn.is_some());
        assert!(!advice.reasons.is_empty());
    }

    #[test]
    fn fill_is_relative_to_window() {
        // The same ~190k carried context is "full" on a 200k window but only
        // ~19% on a 1M window — so a 1M session with stable cache stays healthy.
        let turns: Vec<_> = (0..12)
            .map(|i| {
                if i < 3 {
                    make_turn(i, 100, 100_000, 90_000, 100, vec![])
                } else {
                    make_turn(i, 100, 1_000, 189_000, 100, vec![])
                }
            })
            .collect();
        let on_200k = advise_clear(&turns, DEFAULT_CONTEXT_WINDOW);
        let on_1m = advise_clear(&turns, EXTENDED_CONTEXT_WINDOW);
        assert!(on_200k.context_fraction > on_1m.context_fraction);
        // Stable cache + 95% full (200k) => Consider; same tokens at 19% (1M) => Healthy.
        assert_eq!(on_200k.urgency, ClearUrgency::Consider);
        assert_eq!(on_1m.urgency, ClearUrgency::Healthy);
    }

    #[test]
    fn window_self_corrects_when_hint_too_small() {
        // Carrying 300k tokens is impossible in a 200k window, so even with the
        // default hint the effective window is promoted to 1M and fill stays sane.
        let turns: Vec<_> = (0..8)
            .map(|i| make_turn(i, 100, 1_000, 300_000, 100, vec![]))
            .collect();
        let advice = advise_clear(&turns, DEFAULT_CONTEXT_WINDOW);
        assert_eq!(advice.context_window, EXTENDED_CONTEXT_WINDOW);
        assert!(
            advice.context_fraction <= 1.0,
            "fill should never exceed 100% after self-correction, got {}",
            advice.context_fraction
        );
    }

    #[test]
    fn detect_window_from_model() {
        assert_eq!(
            detect_context_window(Some("claude-opus-4-8[1m]")),
            EXTENDED_CONTEXT_WINDOW
        );
        assert_eq!(
            detect_context_window(Some("claude-opus-4-8")),
            DEFAULT_CONTEXT_WINDOW
        );
        assert_eq!(detect_context_window(None), DEFAULT_CONTEXT_WINDOW);
    }

    #[test]
    fn churn_onset_turn_skips_warmup() {
        let turns: Vec<_> = (0..12)
            .map(|i| make_turn(i, 100, 50_000, 50_000, 100, vec![]))
            .collect();
        let advice = advise_clear(&turns, W);
        // Onset is reported past the warm-up window, never on turn 1-5.
        let onset = advice.churn_onset_turn.expect("expected churn onset");
        assert!(onset > MIN_TURNS, "onset {onset} should be past warm-up");
    }

    #[test]
    fn current_and_peak_context_tracked() {
        let mut turns: Vec<_> = (0..6)
            .map(|i| make_turn(i, 100, 0, 10_000, 100, vec![]))
            .collect();
        // Make the peak occur mid-session, with a smaller final turn.
        turns[3] = make_turn(3, 100, 0, 500_000, 100, vec![]);
        let advice = advise_clear(&turns, W);
        assert_eq!(advice.peak_context_tokens, 500_100);
        assert_eq!(advice.current_context_tokens, 10_100);
    }
}
