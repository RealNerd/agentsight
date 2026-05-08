use std::collections::HashMap;

use crate::parser::types::TurnSummary;

/// Collect per-model turn counts from a session's turns, sorted descending by count.
pub fn collect_model_distribution(turns: &[TurnSummary]) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for turn in turns {
        let model = turn.model.as_deref().unwrap_or("unknown").to_string();
        *counts.entry(model).or_default() += 1;
    }
    let mut result: Vec<(String, usize)> = counts.into_iter().collect();
    result.sort_by_key(|item| std::cmp::Reverse(item.1));
    result
}
