use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static BUNDLED_PROVIDERS: LazyLock<Vec<ProviderDef>> = LazyLock::new(|| {
    let json_data = include_str!("default_providers.json");
    match serde_json::from_str(json_data) {
        Ok(providers) => providers,
        Err(e) => {
            tracing::warn!("Failed to parse default_providers.json: {e}");
            vec![]
        }
    }
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDef {
    pub name: String,
    pub env_vars: Vec<String>,
    pub chat_url: String,
    pub models_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderRegistry {
    providers: Vec<ProviderDef>,
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: BUNDLED_PROVIDERS.clone(),
        }
    }

    pub fn load_or_default(path: Option<&std::path::Path>) -> Self {
        let mut registry = Self::new();

        let Some(p) = path else {
            return registry;
        };

        if !p.exists() {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let default_json = include_str!("default_providers.json");
            let _ = std::fs::write(p, default_json);
            return registry;
        }

        if let Ok(content) = std::fs::read_to_string(p)
            && let Ok(custom_providers) = serde_json::from_str::<Vec<ProviderDef>>(&content)
        {
            // Merge custom providers, overriding defaults based on name
            let mut new_providers = custom_providers;
            for bundled in &registry.providers {
                if !new_providers.iter().any(|p| p.name == bundled.name) {
                    new_providers.push(bundled.clone());
                }
            }
            registry.providers = new_providers;
        }

        registry
    }

    pub fn get_all_providers(&self) -> &[ProviderDef] {
        &self.providers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bundled JSON validation: forces evaluation of the LazyLock and
    /// asserts `default_providers.json` parses into `Vec<ProviderDef>`.
    /// Surfaces a malformed-JSON regression with a clearly named test
    /// instead of as a downstream provider-resolution failure.
    #[test]
    fn bundled_providers_json_parses_into_vec_of_provider_defs() {
        let count = BUNDLED_PROVIDERS.len();
        assert!(
            count > 0,
            "default_providers.json must contain at least one provider"
        );
    }

    #[test]
    fn bundled_providers_have_non_empty_required_fields() {
        for (i, p) in BUNDLED_PROVIDERS.iter().enumerate() {
            assert!(!p.name.is_empty(), "provider[{i}].name is empty");
            assert!(
                !p.chat_url.is_empty(),
                "provider[{i}].chat_url is empty (name={})",
                p.name
            );
        }
    }
}
