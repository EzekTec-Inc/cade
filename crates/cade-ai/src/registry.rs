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

        if let Ok(content) = std::fs::read_to_string(p)
            && let Ok(custom_rules) = serde_json::from_str::<Vec<PricingRule>>(&content)
        {
            // Prepend custom rules so they override defaults
            let mut new_rules = custom_rules;
            new_rules.extend(registry.rules);
            registry.rules = new_rules;
        }

        registry
    }

    /// Returns approximate per-token pricing for a model.
    /// Evaluates rules in order. Unknown models get zero rates.
    pub fn pricing_for_model(&self, model_id: &str) -> ModelPricing {
        // Try resolving against the new llm_providers database first
        let id_clean = model_id.strip_prefix("openrouter/").unwrap_or(model_id);
        let parts: Vec<&str> = id_clean.split('/').collect();
        if parts.len() == 2 {
            let provider = parts[0];
            let model_name = parts[1];
            if let Some(m) = llm_providers::get_model(provider, model_name) {
                let (cache_read, cache_write) = if provider == "anthropic" {
                    (m.input_price * 0.1, m.input_price * 1.25)
                } else if provider == "openai" {
                    (m.input_price * 0.5, 0.0)
                } else if provider == "gemini" || provider == "google" {
                    (m.input_price * 0.25, 0.0)
                } else if provider == "deepseek" {
                    if model_name.contains("reasoner") || model_name.contains("r1") {
                        (m.input_price * 0.25, 0.0) // DeepSeek R1 cache-hits are $0.14 / 1M (~25% of $0.55 base)
                    } else {
                        (m.input_price * 0.1, 0.0) // DeepSeek V3 cache-hits are $0.014 / 1M (exactly 10% of $0.14 base)
                    }
                } else {
                    (0.0, 0.0)
                };

                return ModelPricing {
                    input: m.input_price,
                    output: m.output_price,
                    cache_read,
                    cache_write,
                };
            }
        }

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

    /// Bundled JSON validation: this test fires the `LazyLock` directly
    /// and asserts the embedded `default_pricing.json` parses into the
    /// expected Rust shape.  If a future commit malforms that JSON the
    /// `LazyLock` initialiser will panic the first time *any* code calls
    /// `pricing_for_model`; this dedicated test surfaces the failure
    /// with an obvious name instead of an unrelated downstream test.
    #[test]
    fn bundled_pricing_json_parses_into_vec_of_rules() {
        // Force-evaluate the LazyLock and copy out the length so a
        // panic in the `expect` inside the lazy initialiser would
        // surface here.
        let count = BUNDLED_RULES.len();
        assert!(
            count > 0,
            "default_pricing.json must contain at least one pricing rule"
        );
    }

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
    fn pricing_gpt_5_5() {
        let registry = ModelRegistry::new();
        let p = registry.pricing_for_model("openai/gpt-5.5");
        assert_eq!(p.input, 5.0);
        assert_eq!(p.output, 30.0);
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

    #[test]
    fn pricing_llm_providers_resolves_openai_gpt4o() {
        let registry = ModelRegistry::new();
        let p = registry.pricing_for_model("openai/gpt-4o");
        assert!(p.input > 0.0);
        assert!(p.output > 0.0);
        // Should pull from llm_providers
        assert_eq!(p.input, 2.5); // $2.50 per 1M tokens
        assert_eq!(p.output, 10.0); // $10.00 per 1M tokens
    }

    #[test]
    fn pricing_llm_providers_resolves_anthropic_claude() {
        let registry = ModelRegistry::new();
        let p = registry.pricing_for_model("anthropic/claude-3-5-sonnet-20241022");
        assert!(p.input > 0.0);
        assert!(p.output > 0.0);
        // Cache read should be 10% of input, cache write should be 1.25x of input
        assert_eq!(p.cache_read, p.input * 0.1);
        assert_eq!(p.cache_write, p.input * 1.25);
    }
}
