use crate::parser::types::{SessionSummary, TokenUsage, TurnSummary};

pub fn make_turn(
    index: usize,
    input: u64,
    cache_creation: u64,
    cache_read: u64,
    output: u64,
    tools: Vec<&str>,
) -> TurnSummary {
    TurnSummary {
        index,
        timestamp: None,
        usage: TokenUsage {
            input_tokens: input,
            cache_creation_input_tokens: cache_creation,
            cache_read_input_tokens: cache_read,
            output_tokens: output,
            cache_creation: None,
            service_tier: None,
        },
        tools: tools.into_iter().map(String::from).collect(),
        model: None,
        bash_commands: Vec::new(),
    }
}

pub fn make_turn_with_bash(index: usize, tools: Vec<&str>, commands: Vec<&str>) -> TurnSummary {
    TurnSummary {
        index,
        timestamp: None,
        usage: TokenUsage {
            input_tokens: 100,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            output_tokens: 50,
            cache_creation: None,
            service_tier: None,
        },
        tools: tools.into_iter().map(String::from).collect(),
        model: None,
        bash_commands: commands.into_iter().map(String::from).collect(),
    }
}

pub fn make_session_summary(
    id: &str,
    project: &str,
    cache_read_pct: f64,
    turns_count: usize,
) -> SessionSummary {
    // cache_read_pct is 0.0-1.0 fraction of input that comes from cache reads
    let total_input = 100_000u64;
    let cache_read = (total_input as f64 * cache_read_pct) as u64;
    let input = total_input - cache_read;

    let turns: Vec<TurnSummary> = (0..turns_count)
        .map(|i| {
            let per_turn_input = input / turns_count as u64;
            let per_turn_cache = cache_read / turns_count as u64;
            if i < 3 {
                make_turn(
                    i,
                    per_turn_input,
                    500,
                    per_turn_cache,
                    100,
                    vec!["Read", "Edit"],
                )
            } else {
                make_turn(
                    i,
                    per_turn_input,
                    50,
                    per_turn_cache,
                    100,
                    vec!["Read", "Edit"],
                )
            }
        })
        .collect();

    let mut total_usage = TokenUsage::default();
    for t in &turns {
        total_usage += t.usage.clone();
    }

    SessionSummary {
        session_id: id.to_string(),
        slug: Some(format!("{}-slug", id)),
        project_path: project.to_string(),
        start_time: Some(chrono::Utc::now()),
        end_time: Some(chrono::Utc::now() + chrono::Duration::minutes(30)),
        total_usage,
        turns,
        ..Default::default()
    }
}
