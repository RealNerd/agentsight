use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use std::path::Path;

use crate::config::Config;
use crate::cost::calculate_usage_cost;
use crate::cost::calculator::cache_hit_ratio;
use crate::output;
use crate::output::json::{ConcurrencySlotJson, TimelineJson, TimelineSessionJson};
use crate::output::table::shorten_project;
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index;

pub struct TimelineArgs {
    pub days: u64,
    pub project: Option<String>,
    pub json: bool,
    pub show_cost: bool,
    pub verbose: bool,
}

/// A session prepared for timeline display.
pub struct TimelineSession {
    pub session_id: String,
    pub slug: Option<String>,
    pub project: String,
    pub model: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub duration_minutes: i64,
    pub tokens: u64,
    pub turns: usize,
    pub cache_hit: f64,
    pub cost: Option<f64>,
    /// Per-turn (timestamp, token_count) for accurate burn-rate attribution.
    /// Only turns with timestamps are included. If empty, falls back to
    /// uniform distribution across the session duration.
    pub turn_activity: Vec<(DateTime<Utc>, u64)>,
}

/// A time slot with concurrency count and token volume.
pub struct ConcurrencySlot {
    pub time: DateTime<Utc>,
    pub count: u64,
    pub tokens: u64,
}

/// Compute concurrency slots using a sweep-line algorithm.
///
/// Divides the time axis into slots of `granularity` width, and for each slot
/// counts how many sessions are active and attributes tokens based on actual
/// turn-level activity (not uniform distribution across session lifetime).
pub fn compute_concurrency(
    sessions: &[TimelineSession],
    axis_start: DateTime<Utc>,
    axis_end: DateTime<Utc>,
    granularity: Duration,
) -> Vec<ConcurrencySlot> {
    if granularity.num_seconds() <= 0 {
        return Vec::new();
    }

    let total_seconds = (axis_end - axis_start).num_seconds();
    let slot_seconds = granularity.num_seconds();
    let num_slots = (total_seconds / slot_seconds) as usize + 1;

    let mut slots: Vec<ConcurrencySlot> = (0..num_slots)
        .map(|i| {
            let time = axis_start + granularity * i as i32;
            ConcurrencySlot {
                time,
                count: 0,
                tokens: 0,
            }
        })
        .collect();

    for session in sessions {
        // Concurrency: count session as active from start to end
        for slot in slots.iter_mut() {
            let slot_end = slot.time + granularity;
            if session.start < slot_end && session.end > slot.time {
                slot.count += 1;
            }
        }

        // Token attribution: use per-turn data when available
        if !session.turn_activity.is_empty() {
            // Attribute each turn's tokens to the slot containing its timestamp
            for &(ts, tokens) in &session.turn_activity {
                if ts < axis_start || ts >= axis_end + granularity {
                    continue;
                }
                let offset = (ts - axis_start).num_seconds().max(0);
                let idx = (offset / slot_seconds) as usize;
                if idx < slots.len() {
                    slots[idx].tokens += tokens;
                }
            }
        } else {
            // Fallback: uniform distribution across session duration
            let session_minutes = (session.end - session.start).num_minutes().max(1);
            let tokens_per_minute = session.tokens as f64 / session_minutes as f64;

            for slot in slots.iter_mut() {
                let slot_end = slot.time + granularity;
                if session.start < slot_end && session.end > slot.time {
                    let overlap_start = session.start.max(slot.time);
                    let overlap_end = session.end.min(slot_end);
                    let overlap_minutes = (overlap_end - overlap_start).num_minutes().max(1);
                    slot.tokens += (tokens_per_minute * overlap_minutes as f64) as u64;
                }
            }
        }
    }

    slots
}

/// Determine the time granularity for the given number of days.
fn granularity_for_days(days: u64) -> Duration {
    match days {
        0..=1 => Duration::minutes(30),
        2..=3 => Duration::hours(1),
        4..=14 => Duration::hours(4),
        _ => Duration::days(1),
    }
}

pub fn run(claude_dir: &Path, config: &Config, args: &TimelineArgs) -> Result<()> {
    let session_files = session_index::discover_sessions(claude_dir)?;

    let cutoff = Utc::now() - Duration::days(args.days as i64);

    let mut sessions: Vec<TimelineSession> = Vec::new();

    for sf in &session_files {
        let project_path = decode_project_path(&sf.project_dir_name);

        if let Some(ref filter) = args.project {
            if !project_path.contains(filter.as_str()) {
                continue;
            }
        }

        let entries = reader::parse_session_file(&sf.path, args.verbose)?;
        let summary = reader::summarize_session(&entries, sf.session_id.clone(), project_path);

        // Require both start and end times for timeline
        let (start, end) = match (summary.start_time, summary.end_time) {
            (Some(s), Some(e)) => (s, e),
            _ => continue,
        };

        if start < cutoff {
            continue;
        }

        let pricing = lookup_pricing(config, &summary);
        let cost_breakdown = calculate_usage_cost(&summary.total_usage, &pricing);
        let hit = cache_hit_ratio(&summary.total_usage);

        let cost_val = if args.show_cost {
            Some(cost_breakdown.total())
        } else {
            None
        };

        let turn_activity: Vec<(DateTime<Utc>, u64)> = summary
            .turns
            .iter()
            .filter_map(|t| t.timestamp.map(|ts| (ts, t.usage.total_tokens())))
            .collect();

        sessions.push(TimelineSession {
            session_id: summary.session_id,
            slug: summary.slug,
            project: shorten_project(&summary.project_path),
            model: summary.model,
            start,
            end,
            duration_minutes: (end - start).num_minutes(),
            tokens: summary.total_usage.total_tokens(),
            turns: summary.turns.len(),
            cache_hit: hit,
            cost: cost_val,
            turn_activity,
        });
    }

    // Sort by start time
    sessions.sort_by_key(|s| s.start);

    if sessions.is_empty() {
        if args.json {
            let now = Utc::now();
            print_timeline_json(&[], &[], now, now, args.days);
        } else {
            println!(
                "No sessions with time data found in the last {} day(s).",
                args.days
            );
        }
        return Ok(());
    }

    // Determine axis bounds
    let axis_start = sessions.iter().map(|s| s.start).min().unwrap();
    let axis_end = sessions.iter().map(|s| s.end).max().unwrap();

    let granularity = granularity_for_days(args.days);
    let concurrency = compute_concurrency(&sessions, axis_start, axis_end, granularity);

    if args.json {
        print_timeline_json(&sessions, &concurrency, axis_start, axis_end, args.days);
    } else {
        output::table::render_timeline(
            &sessions,
            &concurrency,
            axis_start,
            axis_end,
            granularity,
            args.show_cost,
        );

        // Summary footer
        let peak = concurrency.iter().map(|s| s.count).max().unwrap_or(0);
        let total_tokens: u64 = sessions.iter().map(|s| s.tokens).sum();

        println!();
        if peak > 0 {
            // Find peak time slot
            if let Some(peak_slot) = concurrency.iter().find(|s| s.count == peak) {
                println!(
                    " Peak concurrency: {} sessions ({})",
                    peak,
                    peak_slot.time.format("%H:%M")
                );
            }
        }

        if args.show_cost {
            let total_cost: f64 = sessions.iter().filter_map(|s| s.cost).sum();
            println!(
                " Total: {} tokens, {} across {} sessions",
                output::format_tokens(total_tokens),
                output::format_cost(total_cost),
                sessions.len()
            );
        } else {
            println!(
                " Total: {} tokens across {} sessions",
                output::format_tokens(total_tokens),
                sessions.len()
            );
        }
    }

    Ok(())
}

fn print_timeline_json(
    sessions: &[TimelineSession],
    concurrency: &[ConcurrencySlot],
    axis_start: DateTime<Utc>,
    axis_end: DateTime<Utc>,
    days: u64,
) {
    let peak = concurrency.iter().map(|s| s.count).max().unwrap_or(0);

    let timeline = TimelineJson {
        period_start: axis_start.to_rfc3339(),
        period_end: axis_end.to_rfc3339(),
        period_days: days,
        sessions: sessions
            .iter()
            .map(|s| TimelineSessionJson {
                session_id: s.session_id.clone(),
                slug: s.slug.clone(),
                project: s.project.clone(),
                model: s.model.clone(),
                start_time: s.start.to_rfc3339(),
                end_time: s.end.to_rfc3339(),
                duration_minutes: s.duration_minutes,
                tokens: s.tokens,
                turns: s.turns,
                cache_hit_ratio: s.cache_hit,
                cost: s.cost,
            })
            .collect(),
        concurrency: concurrency
            .iter()
            .map(|c| ConcurrencySlotJson {
                time: c.time.to_rfc3339(),
                count: c.count,
                tokens: c.tokens,
            })
            .collect(),
        peak_concurrent: peak,
        total_sessions: sessions.len(),
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&timeline).unwrap_or_default()
    );
}

fn lookup_pricing(
    config: &Config,
    summary: &crate::parser::types::SessionSummary,
) -> crate::config::ModelPricing {
    let model_name = summary.model.as_deref().unwrap_or("claude-opus-4-6");
    config
        .pricing_for_model(model_name)
        .cloned()
        .unwrap_or(crate::config::ModelPricing {
            input_per_million: 5.0,
            output_per_million: 25.0,
            cache_creation_per_million: 6.25,
            cache_read_per_million: 0.5,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session(
        id: &str,
        project: &str,
        start_offset_min: i64,
        duration_min: i64,
        tokens: u64,
    ) -> TimelineSession {
        let base = Utc::now() - Duration::hours(4);
        let start = base + Duration::minutes(start_offset_min);
        let end = base + Duration::minutes(start_offset_min + duration_min);
        // Default: spread turns evenly across the session for test predictability
        let num_turns = 5;
        let tokens_per_turn = tokens / num_turns;
        let turn_activity: Vec<(DateTime<Utc>, u64)> = (0..num_turns)
            .map(|i| {
                let t = start + Duration::minutes((i as i64 * duration_min) / num_turns as i64);
                (t, tokens_per_turn)
            })
            .collect();
        TimelineSession {
            session_id: id.to_string(),
            slug: Some(id.to_string()),
            project: project.to_string(),
            model: Some("claude-opus-4-6".to_string()),
            start,
            end,
            duration_minutes: duration_min,
            tokens,
            turns: num_turns as usize,
            cache_hit: 0.85,
            cost: None,
            turn_activity,
        }
    }

    #[test]
    fn test_compute_concurrency_single_session() {
        let sessions = vec![make_session("a", "proj", 0, 60, 10000)];
        let start = sessions[0].start;
        let end = sessions[0].end;
        let gran = Duration::minutes(30);

        let slots = compute_concurrency(&sessions, start, end, gran);
        assert!(!slots.is_empty());
        // Slots covering the session interval should have count 1
        // The last slot starts at end time, so the session doesn't overlap it
        assert_eq!(slots[0].count, 1);
        assert_eq!(slots[1].count, 1);
        // Slot at exact end time: session.end == slot.time, so no overlap
        assert_eq!(slots[2].count, 0);
    }

    #[test]
    fn test_compute_concurrency_overlap() {
        let sessions = vec![
            make_session("a", "proj1", 0, 120, 12000),
            make_session("b", "proj2", 30, 60, 6000),
        ];
        let start = sessions[0].start;
        let end = sessions[0].end;
        let gran = Duration::minutes(30);

        let slots = compute_concurrency(&sessions, start, end, gran);

        // First slot (0-30): only session a → count 1
        assert_eq!(slots[0].count, 1);
        // Second slot (30-60): both sessions → count 2
        assert_eq!(slots[1].count, 2);
        // Third slot (60-90): session a only (b ends at 90 but started at 30, so 30+60=90)
        // Actually b runs from 30 to 90, and slot 2 is 60-90, so b is still active
        assert_eq!(slots[2].count, 2);
    }

    #[test]
    fn test_compute_concurrency_no_sessions() {
        let now = Utc::now();
        let slots = compute_concurrency(&[], now, now + Duration::hours(1), Duration::minutes(30));
        // Should still produce slots, just with zero counts
        for slot in &slots {
            assert_eq!(slot.count, 0);
            assert_eq!(slot.tokens, 0);
        }
    }

    #[test]
    fn test_tokens_not_attributed_during_idle_gap() {
        // Simulate an overnight session: active 0-30min and 480-510min, idle in between
        let base = Utc::now() - Duration::hours(10);
        let session = TimelineSession {
            session_id: "overnight".to_string(),
            slug: Some("overnight".to_string()),
            project: "test".to_string(),
            model: None,
            start: base,
            end: base + Duration::minutes(510),
            duration_minutes: 510,
            tokens: 20000,
            turns: 4,
            cache_hit: 0.8,
            cost: None,
            // All activity in first 30 min and last 30 min — nothing overnight
            turn_activity: vec![
                (base + Duration::minutes(5), 5000),
                (base + Duration::minutes(20), 5000),
                (base + Duration::minutes(485), 5000),
                (base + Duration::minutes(500), 5000),
            ],
        };

        let sessions = vec![session];
        let gran = Duration::minutes(30);
        let slots = compute_concurrency(&sessions, base, base + Duration::minutes(510), gran);

        // Slot at minute 0 (0-30): should have tokens from turns at min 5 and 20
        assert_eq!(slots[0].tokens, 10000);
        // Slot at minute 30 (30-60): no turns → 0 tokens
        assert_eq!(slots[1].tokens, 0);
        // Slot at minute 60 (60-90): no turns → 0 tokens
        assert_eq!(slots[2].tokens, 0);
        // Middle slots should all be 0
        for slot in &slots[3..15] {
            assert_eq!(
                slot.tokens, 0,
                "idle slot at {} should have 0 tokens",
                slot.time
            );
        }
        // Slot at minute 480 (480-510): should have tokens from turns at 485 and 500
        let slot_480_idx = (480 / 30) as usize;
        assert_eq!(slots[slot_480_idx].tokens, 10000);

        // But concurrency count should be 1 for all slots in the session range
        assert_eq!(slots[0].count, 1);
        assert_eq!(slots[5].count, 1);
        assert_eq!(slots[slot_480_idx].count, 1);
    }

    #[test]
    fn test_granularity_for_days() {
        assert_eq!(granularity_for_days(1).num_minutes(), 30);
        assert_eq!(granularity_for_days(2).num_hours(), 1);
        assert_eq!(granularity_for_days(7).num_hours(), 4);
        assert_eq!(granularity_for_days(30).num_hours(), 24);
    }
}
