use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Compile the default pricing config into the binary.
const DEFAULT_PRICING_TOML: &str = include_str!("../../config/default_pricing.toml");

/// Billing mode determines whether cost columns are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingMode {
    /// Subscription user (Claude Max, Team, etc.) — show usage only by default.
    Max,
    /// API/pay-per-token user — show usage and cost.
    Api,
}

impl BillingMode {
    pub fn show_cost(self) -> bool {
        self == BillingMode::Api
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub models: HashMap<String, ModelPricing>,
    /// "max" or "api" — controls whether cost is shown by default.
    #[serde(default)]
    pub billing: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cache_creation_per_million: f64,
    pub cache_read_per_million: f64,
}

impl Config {
    /// Load config: start with compiled-in defaults, merge user overrides if present.
    pub fn load(override_path: Option<&Path>) -> Result<Self> {
        let mut config: Config = toml::from_str(DEFAULT_PRICING_TOML)?;

        // Check for user override file
        let override_file = override_path
            .map(PathBuf::from)
            .or_else(default_config_path);

        if let Some(path) = override_file {
            if path.exists() {
                let contents = std::fs::read_to_string(&path)?;
                let user_config: Config = toml::from_str(&contents)?;
                // Merge: user config wins
                if user_config.billing.is_some() {
                    config.billing = user_config.billing;
                }
                for (model, pricing) in user_config.models {
                    config.models.insert(model, pricing);
                }
            }
        }

        Ok(config)
    }

    /// Resolve billing mode from config file setting.
    pub fn billing_mode(&self) -> BillingMode {
        match self.billing.as_deref() {
            Some("api") => BillingMode::Api,
            _ => BillingMode::Max, // Default to Max (usage-only)
        }
    }

    /// Look up pricing for a model name. Tries exact match first, then prefix match.
    pub fn pricing_for_model(&self, model: &str) -> Option<&ModelPricing> {
        // Exact match
        if let Some(p) = self.models.get(model) {
            return Some(p);
        }

        // Prefix match: "claude-opus-4-6" matches "claude-opus-4-6-20260401"
        for (key, pricing) in &self.models {
            if model.starts_with(key.as_str()) {
                return Some(pricing);
            }
        }

        None
    }
}

/// Default path for user config: ~/.agentsight/config.toml
fn default_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".agentsight").join("config.toml"))
}

/// Default path for Claude Code data: ~/.claude
pub fn default_claude_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".claude"))
        .unwrap_or_else(|| PathBuf::from(".claude"))
}
