use crate::cost::calculator::cache_hit_ratio;
use crate::parser::types::SessionSummary;

use super::types::{ProjectTrend, SessionTrendPoint, TrendDirection};

/// Analyze cache hit trend across sessions for a project.
/// Sessions should be sorted by time (oldest first).
pub fn analyze_project_trend(summaries: &[SessionSummary]) -> ProjectTrend {
    let points: Vec<SessionTrendPoint> = summaries
        .iter()
        .map(|s| {
            let hit = cache_hit_ratio(&s.total_usage);
            let diag = super::run_diagnose(s);
            SessionTrendPoint {
                session_id: s.session_id.clone(),
                slug: s.slug.clone(),
                date: s.start_time.map(|t| t.format("%Y-%m-%d %H:%M").to_string()),
                tokens: s.total_usage.total_tokens(),
                cache_hit: hit,
                classification: diag.cache_stability.classification,
            }
        })
        .collect();

    let overall_avg = if points.is_empty() {
        0.0
    } else {
        points.iter().map(|p| p.cache_hit).sum::<f64>() / points.len() as f64
    };

    let recent_count = 5.min(points.len());
    let recent_avg = if recent_count == 0 {
        0.0
    } else {
        points[points.len() - recent_count..]
            .iter()
            .map(|p| p.cache_hit)
            .sum::<f64>()
            / recent_count as f64
    };

    let direction = if points.len() < 3 {
        TrendDirection::Stable
    } else {
        let diff = recent_avg - overall_avg;
        if diff > 0.05 {
            TrendDirection::Improving
        } else if diff < -0.05 {
            TrendDirection::Declining
        } else {
            TrendDirection::Stable
        }
    };

    ProjectTrend {
        points,
        direction,
        recent_avg_cache_hit: recent_avg,
        overall_avg_cache_hit: overall_avg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::diagnose::test_helpers::make_session_summary;

    #[test]
    fn test_analyze_project_trend_stable() {
        let summaries: Vec<_> = (0..5)
            .map(|i| make_session_summary(&format!("s{}", i), "/foo/bar", 0.85, 10))
            .collect();

        let trend = analyze_project_trend(&summaries);
        assert_eq!(trend.direction, TrendDirection::Stable);
        assert_eq!(trend.points.len(), 5);
    }

    #[test]
    fn test_analyze_project_trend_improving() {
        let mut summaries: Vec<_> = Vec::new();
        for i in 0..8 {
            let cache = if i < 3 { 0.50 } else { 0.92 };
            summaries.push(make_session_summary(
                &format!("s{}", i),
                "/foo/bar",
                cache,
                10,
            ));
        }

        let trend = analyze_project_trend(&summaries);
        assert_eq!(trend.direction, TrendDirection::Improving);
    }

    #[test]
    fn test_analyze_project_trend_declining() {
        let mut summaries: Vec<_> = Vec::new();
        for i in 0..8 {
            let cache = if i < 3 { 0.92 } else { 0.50 };
            summaries.push(make_session_summary(
                &format!("s{}", i),
                "/foo/bar",
                cache,
                10,
            ));
        }

        let trend = analyze_project_trend(&summaries);
        assert_eq!(trend.direction, TrendDirection::Declining);
    }
}
