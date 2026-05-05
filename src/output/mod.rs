pub mod json;
pub mod table;

/// Format a dollar amount for display.
pub fn format_cost(cost: f64) -> String {
    if cost < 0.01 {
        format!("${:.4}", cost)
    } else {
        format!("${:.2}", cost)
    }
}

/// Format a token count with thousands separators.
pub fn format_tokens(count: u64) -> String {
    let s = count.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

/// Format a percentage for display.
pub fn format_percent(ratio: f64) -> String {
    format!("{:.1}%", ratio * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_cost() {
        assert_eq!(format_cost(4.308), "$4.31");
        assert_eq!(format_cost(0.001), "$0.0010");
        assert_eq!(format_cost(0.0), "$0.0000");
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_000), "1,000");
        assert_eq!(format_tokens(1_234_567), "1,234,567");
    }

    #[test]
    fn test_format_percent() {
        assert_eq!(format_percent(0.706), "70.6%");
        assert_eq!(format_percent(0.0), "0.0%");
        assert_eq!(format_percent(1.0), "100.0%");
    }
}
