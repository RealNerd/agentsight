use crate::parser::types::TurnSummary;

use super::types::ContextGrowth;

/// Track total input tokens per turn and flag if input grows >2x from turn 5 to final.
pub fn analyze_context_growth(turns: &[TurnSummary]) -> ContextGrowth {
    let per_turn_input: Vec<u64> = turns
        .iter()
        .map(|t| {
            t.usage.input_tokens
                + t.usage.cache_creation_input_tokens
                + t.usage.cache_read_input_tokens
        })
        .collect();

    let (growth_factor, flagged) = if per_turn_input.len() > 5 {
        let turn5_input = per_turn_input[4];
        let final_input = *per_turn_input.last().unwrap_or(&0);
        if turn5_input > 0 {
            let factor = final_input as f64 / turn5_input as f64;
            (factor, factor > 2.0)
        } else {
            (0.0, false)
        }
    } else {
        (0.0, false)
    };

    ContextGrowth {
        growth_factor,
        flagged,
        per_turn_input,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::diagnose::test_helpers::make_turn;

    #[test]
    fn test_context_growth_flagged() {
        // Input grows >2x from turn 5 to final -> flagged
        let turns: Vec<_> = (0..10)
            .map(|i| {
                let input = 10_000 * (1 + i as u64 * i as u64); // quadratic growth
                make_turn(i, input, 0, 0, 100, vec![])
            })
            .collect();

        let result = analyze_context_growth(&turns);
        assert!(result.flagged);
        assert!(result.growth_factor > 2.0);
    }

    #[test]
    fn test_context_growth_flat() {
        // Stable input -> not flagged
        let turns: Vec<_> = (0..10)
            .map(|i| make_turn(i, 10_000, 0, 0, 100, vec![]))
            .collect();

        let result = analyze_context_growth(&turns);
        assert!(!result.flagged);
        assert!((result.growth_factor - 1.0).abs() < 0.01);
    }
}
