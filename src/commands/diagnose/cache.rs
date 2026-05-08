use crate::parser::types::TurnSummary;

use super::types::{CacheClassification, CacheStability};

/// Compute per-turn cache creation ratio and classify the session's cache behavior.
///
/// - Stable: cache creation >30% on early turns, then drops below 15%
/// - Churning: cache creation stays >30% past turn 5
/// - Degrading: second-half average exceeds first-half average
pub fn analyze_cache_stability(turns: &[TurnSummary]) -> CacheStability {
    if turns.len() < 5 {
        return CacheStability {
            classification: CacheClassification::Stable,
            turns_above_threshold: 0,
            total_turns: turns.len(),
            avg_cache_creation_pct: 0.0,
            per_turn_ratios: Vec::new(),
        };
    }

    let per_turn_ratios: Vec<f64> = turns
        .iter()
        .map(|t| {
            let total = t.usage.input_tokens
                + t.usage.cache_creation_input_tokens
                + t.usage.cache_read_input_tokens;
            if total == 0 {
                0.0
            } else {
                t.usage.cache_creation_input_tokens as f64 / total as f64
            }
        })
        .collect();

    let turns_above_threshold = per_turn_ratios.iter().filter(|r| **r > 0.30).count();

    let avg_cache_creation_pct = if per_turn_ratios.is_empty() {
        0.0
    } else {
        per_turn_ratios.iter().sum::<f64>() / per_turn_ratios.len() as f64 * 100.0
    };

    let mid = per_turn_ratios.len() / 2;
    let first_half = &per_turn_ratios[..mid];
    let second_half = &per_turn_ratios[mid..];

    let first_avg = if first_half.is_empty() {
        0.0
    } else {
        first_half.iter().sum::<f64>() / first_half.len() as f64
    };
    let second_avg = if second_half.is_empty() {
        0.0
    } else {
        second_half.iter().sum::<f64>() / second_half.len() as f64
    };

    let classification = if second_avg > first_avg && second_avg > 0.15 {
        CacheClassification::Degrading
    } else if turns_above_threshold > 5 {
        CacheClassification::Churning
    } else {
        CacheClassification::Stable
    };

    CacheStability {
        classification,
        turns_above_threshold,
        total_turns: turns.len(),
        avg_cache_creation_pct,
        per_turn_ratios,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::diagnose::test_helpers::make_turn;

    #[test]
    fn test_cache_stability_stable() {
        // High early cache creation that drops off -> Stable
        let turns: Vec<_> = (0..10)
            .map(|i| {
                if i < 3 {
                    make_turn(i, 100, 600, 300, 50, vec![])
                } else {
                    make_turn(i, 100, 50, 850, 50, vec![])
                }
            })
            .collect();

        let result = analyze_cache_stability(&turns);
        assert_eq!(result.classification, CacheClassification::Stable);
        assert_eq!(result.total_turns, 10);
    }

    #[test]
    fn test_cache_stability_churning() {
        // Sustained high cache creation across all turns -> Churning
        let turns: Vec<_> = (0..10)
            .map(|i| make_turn(i, 100, 500, 400, 50, vec![]))
            .collect();

        let result = analyze_cache_stability(&turns);
        assert_eq!(result.classification, CacheClassification::Churning);
        assert!(result.turns_above_threshold > 5);
    }

    #[test]
    fn test_cache_stability_degrading() {
        // Cache creation increases over the session -> Degrading
        let turns: Vec<_> = (0..10)
            .map(|i| {
                if i < 5 {
                    make_turn(i, 100, 50, 850, 50, vec![])
                } else {
                    make_turn(i, 100, 400, 500, 50, vec![])
                }
            })
            .collect();

        let result = analyze_cache_stability(&turns);
        assert_eq!(result.classification, CacheClassification::Degrading);
    }

    #[test]
    fn test_cache_stability_short_session() {
        // <5 turns -> Stable (too short to classify)
        let turns: Vec<_> = (0..3)
            .map(|i| make_turn(i, 100, 500, 400, 50, vec![]))
            .collect();

        let result = analyze_cache_stability(&turns);
        assert_eq!(result.classification, CacheClassification::Stable);
        assert_eq!(result.total_turns, 3);
    }
}
