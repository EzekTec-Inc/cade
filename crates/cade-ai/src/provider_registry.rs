use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static BUNDLED_PROVIDERS: LazyLock<Vec<ProviderDef>> = LazyLock::new(|| {
    let json_data = include_str!("default_providers.json");
    serde_json::from_str(json_data).expect("Failed to parse default_providers.json")
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
