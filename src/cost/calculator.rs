use crate::config::ModelPricing;
use crate::parser::types::TokenUsage;

/// Dollar amounts broken down by token bucket.
#[derive(Debug, Default, Clone)]
pub struct CostBreakdown {
    pub input_cost: f64,
    pub cache_creation_cost: f64,
    pub cache_read_cost: f64,
    pub output_cost: f64,
}

impl CostBreakdown {
    pub fn total(&self) -> f64 {
        self.input_cost + self.cache_creation_cost + self.cache_read_cost + self.output_cost
    }

}

impl std::ops::AddAssign for CostBreakdown {
    fn add_assign(&mut self, rhs: Self) {
        self.input_cost += rhs.input_cost;
        self.cache_creation_cost += rhs.cache_creation_cost;
        self.cache_read_cost += rhs.cache_read_cost;
        self.output_cost += rhs.output_cost;
    }
}

/// Calculate cost for a TokenUsage given model pricing.
pub fn calculate_usage_cost(usage: &TokenUsage, pricing: &ModelPricing) -> CostBreakdown {
    CostBreakdown {
        input_cost: usage.input_tokens as f64 * pricing.input_per_million / 1_000_000.0,
        cache_creation_cost: usage.cache_creation_input_tokens as f64
            * pricing.cache_creation_per_million
            / 1_000_000.0,
        cache_read_cost: usage.cache_read_input_tokens as f64 * pricing.cache_read_per_million
            / 1_000_000.0,
        output_cost: usage.output_tokens as f64 * pricing.output_per_million / 1_000_000.0,
    }
}

/// Cache hit ratio: what fraction of all input tokens came from cache reads.
pub fn cache_hit_ratio(usage: &TokenUsage) -> f64 {
    let total_input =
        usage.input_tokens + usage.cache_creation_input_tokens + usage.cache_read_input_tokens;
    if total_input == 0 {
        return 0.0;
    }
    usage.cache_read_input_tokens as f64 / total_input as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opus_pricing() -> ModelPricing {
        ModelPricing {
            input_per_million: 5.00,
            output_per_million: 25.00,
            cache_creation_per_million: 6.25,
            cache_read_per_million: 0.50,
        }
    }

    #[test]
    fn test_basic_cost_calculation() {
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            output_tokens: 100_000,
            cache_creation: None,
            service_tier: None,
        };

        let cost = calculate_usage_cost(&usage, &opus_pricing());
        assert!((cost.input_cost - 5.00).abs() < 0.001);
        assert!((cost.output_cost - 2.50).abs() < 0.001);
        assert!((cost.total() - 7.50).abs() < 0.001);
    }

    #[test]
    fn test_cache_cost_calculation() {
        let usage = TokenUsage {
            input_tokens: 100,
            cache_creation_input_tokens: 10_000,
            cache_read_input_tokens: 50_000,
            output_tokens: 500,
            cache_creation: None,
            service_tier: None,
        };

        let cost = calculate_usage_cost(&usage, &opus_pricing());
        assert!((cost.cache_creation_cost - 0.0625).abs() < 0.0001);
        assert!((cost.cache_read_cost - 0.025).abs() < 0.0001);
    }

    #[test]
    fn test_zero_usage() {
        let usage = TokenUsage::default();
        let cost = calculate_usage_cost(&usage, &opus_pricing());
        assert!((cost.total()).abs() < 0.0001);
    }

    #[test]
    fn test_cache_hit_ratio() {
        let usage = TokenUsage {
            input_tokens: 100,
            cache_creation_input_tokens: 10_000,
            cache_read_input_tokens: 40_000,
            output_tokens: 500,
            cache_creation: None,
            service_tier: None,
        };

        let ratio = cache_hit_ratio(&usage);
        // 40000 / (100 + 10000 + 40000) = 40000 / 50100 ≈ 0.798
        assert!((ratio - 0.798).abs() < 0.01);
    }

    #[test]
    fn test_cache_hit_ratio_zero_input() {
        let usage = TokenUsage::default();
        assert!((cache_hit_ratio(&usage)).abs() < 0.0001);
    }
}
