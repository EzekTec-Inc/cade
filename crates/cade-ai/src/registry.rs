use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static BUNDLED_RULES: LazyLock<Vec<PricingRule>> = LazyLock::new(|| {
    let json_data = include_str!("default_pricing.json");
    serde_json::from_str(json_data).expect("Failed to parse default_pricing.json")
});

/// Pricing data per 1M tokens.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelPricing {
    pub input: f64,       // $/1M input tokens
    pub output: f64,      // $/1M output tokens
    pub cache_read: f64,  // $/1M cache-read tokens
    pub cache_write: f64, // $/1M cache-write tokens
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingRule {
    #[serde(default)]
    pub contains_any: Vec<String>,
    #[serde(default)]
    pub starts_with_any: Vec<String>,
    #[serde(default)]
    pub not_contains_any: Vec<String>,
    pub pricing: ModelPricing,
}

impl PricingRule {
    fn matches(&self, model_id: &str) -> bool {
        if !self.not_contains_any.is_empty()
            && self.not_contains_any.iter().any(|nc| model_id.contains(nc))
        {
            return false;
        }

        let has_contains = !self.contains_any.is_empty();
        let has_starts = !self.starts_with_any.is_empty();

        if !has_contains && !has_starts {
            return true;
        }

        if has_contains && self.contains_any.iter().any(|c| model_id.contains(c)) {
            return true;
        }
        if has_starts && self.starts_with_any.iter().any(|s| model_id.starts_with(s)) {
            return true;
        }

        false
    }
}

/// A registry that holds dynamic pricing data for models.
#[derive(Debug, Clone)]
pub struct ModelRegistry {
    rules: Vec<PricingRule>,
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelRegistry {
    /// Create a new ModelRegistry with default bundled pricing.
    pub fn new() -> Self {
        Self {
            rules: BUNDLED_RULES.clone(),
        }
    }

    /// Load pricing rules from a file, creating it with defaults if it doesn't exist.
    /// Custom rules are prepended to the bundled rules.
    pub fn load_or_default(path: Option<&std::path::Path>) -> Self {
        let mut registry = Self::new();
        
        let Some(p) = path else {
            return registry;
        };

        if !p.exists() {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let default_json = include_str!("default_pricing.json");
            let _ = std::fs::write(p, default_json);
            return registry;
        }

        if let Ok(content) = std::fs::read_to_string(p) {
            if let Ok(custom_rules) = serde_json::from_str::<Vec<PricingRule>>(&content) {
                // Prepend custom rules so they override defaults
                let mut new_rules = custom_rules;
                new_rules.extend(registry.rules);
                registry.rules = new_rules;
            }
        }

        registry
    }

    /// Returns approximate per-token pricing for a model.
    /// Evaluates rules in order. Unknown models get zero rates.
    pub fn pricing_for_model(&self, model_id: &str) -> ModelPricing {
        for rule in &self.rules {
            if rule.matches(model_id) {
                return rule.pricing.clone();
            }
        }
        ModelPricing::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pricing_claude_sonnet() {
        let registry = ModelRegistry::new();
        let p = registry.pricing_for_model("anthropic/claude-sonnet-4-5-20250929");
        assert!(p.input > 0.0);
        assert!(p.output > 0.0);
        assert!(p.cache_read > 0.0);
        assert!(p.cache_write > 0.0);
    }

    #[test]
    fn pricing_gpt_4o() {
        let registry = ModelRegistry::new();
        let p = registry.pricing_for_model("openai/gpt-4o");
        assert!(p.input > 0.0);
        assert!(p.output > 0.0);
    }

    #[test]
    fn pricing_gemini_25_pro() {
        let registry = ModelRegistry::new();
        let p = registry.pricing_for_model("gemini/gemini-2.5-pro");
        assert!(p.input > 0.0);
        assert!(p.output > 0.0);
    }

    #[test]
    fn pricing_unknown_model_zero() {
        let registry = ModelRegistry::new();
        let p = registry.pricing_for_model("random/model-xyz");
        assert_eq!(p.input, 0.0);
        assert_eq!(p.output, 0.0);
        assert_eq!(p.cache_read, 0.0);
        assert_eq!(p.cache_write, 0.0);
    }

    #[test]
    fn pricing_provider_prefix_fallback() {
        // Unknown anthropic model should get the fallback anthropic pricing
        let registry = ModelRegistry::new();
        let p = registry.pricing_for_model("anthropic/future-model");
        assert!(p.input > 0.0);
    }

    #[test]
    fn pricing_custom_override() {
        use std::io::Write;
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        let custom_json = r#"[
            {
                "contains_any": ["custom-model-1"],
                "pricing": { "input": 99.0, "output": 99.0, "cache_read": 99.0, "cache_write": 99.0 }
            }
        ]"#;
        temp_file.write_all(custom_json.as_bytes()).unwrap();

        let registry = ModelRegistry::load_or_default(Some(temp_file.path()));
        let p = registry.pricing_for_model("custom-model-1");
        assert_eq!(p.input, 99.0);
    }
}
