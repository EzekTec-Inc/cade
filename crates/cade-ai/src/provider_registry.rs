use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default = "compat_kind")]
    pub kind: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub env_vars: Vec<String>,
    pub chat_url: String,
    #[serde(default)]
    pub models_url: Option<String>,
    #[serde(default = "discovery_enabled")]
    pub discovery: bool,
    #[serde(default)]
    pub base_url_env: Vec<String>,
    /// Optional binding to a legacy AiConfig credential slot; custom definitions
    /// otherwise use their own env_vars without inheriting another provider's key.
    #[serde(default)]
    pub config_key: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub fast_model: Option<String>,
    /// Optional background/compaction choice; absent values use the configured
    /// fast choice, then the declared default. IDs are upstream identifiers.
    #[serde(default)]
    pub background_model: Option<String>,
    /// Optional namespace used by the independent offline pricing database.
    #[serde(default)]
    pub pricing_provider: Option<String>,
    #[serde(default = "default_priority")]
    pub priority: u32,
    #[serde(default = "default_priority")]
    pub fast_priority: u32,
    #[serde(default)]
    pub local: bool,
}

fn compat_kind() -> String {
    "openai-compatible".into()
}
fn discovery_enabled() -> bool {
    true
}
fn default_priority() -> u32 {
    100
}

impl ProviderDef {
    pub fn env_key(&self) -> Option<String> {
        self.env_vars
            .iter()
            .filter(|v| valid_env_name(v))
            .find_map(|v| std::env::var(v).ok().filter(|k| !k.trim().is_empty()))
    }

    pub fn endpoint(&self) -> String {
        self.base_url_env
            .iter()
            .filter(|v| valid_env_name(v))
            .find_map(|v| std::env::var(v).ok().filter(|s| !s.trim().is_empty()))
            .unwrap_or_else(|| self.chat_url.clone())
    }
}

pub(crate) fn valid_provider_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.chars().any(|c| c.is_whitespace() || c.is_control())
}

pub(crate) fn valid_env_name(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(|c| matches!(c, '=' | '\0'))
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
    pub fn configured() -> Self {
        Self::load_or_default(
            crate::runtime::config_path("CADE_PROVIDERS_CONFIG", "providers.json").as_deref(),
        )
    }

    pub fn get(&self, name: &str) -> Option<&ProviderDef> {
        self.providers.iter().find(|p| p.name == name).or_else(|| {
            self.providers
                .iter()
                .find(|p| p.aliases.iter().any(|a| a == name))
        })
    }

    pub fn alias_map(&self) -> BTreeMap<String, String> {
        self.providers
            .iter()
            .flat_map(|provider| {
                provider
                    .aliases
                    .iter()
                    .filter(|alias| {
                        !self
                            .providers
                            .iter()
                            .any(|p| p.name.as_str() == alias.as_str())
                    })
                    .map(|alias| (alias.clone(), provider.name.clone()))
            })
            .collect()
    }

    pub fn detected_default(&self) -> Option<&ProviderDef> {
        self.providers
            .iter()
            .filter(|p| p.local || p.env_key().is_some())
            .min_by_key(|p| (p.priority, &p.name))
    }

    /// Definitions store upstream IDs; qualify them with the actual registered
    /// provider name, preserving nested deployment namespaces.
    pub fn default_model_id_for(&self, provider: &str) -> Option<String> {
        let definition = self.get(provider)?;
        definition
            .default_model
            .as_ref()
            .filter(|model| !model.trim().is_empty())
            .map(|model| format!("{}/{model}", definition.name))
    }

    pub fn fast_model_for(
        &self,
        primary: &str,
        metadata: &crate::runtime::RuntimeRegistry,
    ) -> String {
        self.auxiliary_model_for(primary, metadata, false)
    }

    pub fn background_model_for(
        &self,
        primary: &str,
        metadata: &crate::runtime::RuntimeRegistry,
    ) -> String {
        self.auxiliary_model_for(primary, metadata, true)
    }

    fn auxiliary_model_for(
        &self,
        primary: &str,
        metadata: &crate::runtime::RuntimeRegistry,
        background: bool,
    ) -> String {
        // Keep an explicit transport namespace (including broker/nested IDs).
        // Canonical lookup normalization is only needed for bare models/aliases.
        let normalized;
        let (owner, model) = match primary.split_once('/') {
            Some(parts) => parts,
            None => {
                normalized = metadata.canonical_model_id(primary);
                match normalized.split_once('/') {
                    Some(parts) => parts,
                    None => return primary.into(),
                }
            }
        };
        let Some(provider) = self.get(owner) else {
            return primary.into();
        };
        if background
            && metadata
                .metadata(&provider.name, model)
                .prefer_primary_for_background
                == Some(true)
        {
            return primary.into();
        }
        let choice = if background {
            provider
                .background_model
                .as_ref()
                .or(provider.fast_model.as_ref())
                .or(provider.default_model.as_ref())
        } else {
            provider.fast_model.as_ref()
        };
        choice
            .filter(|model| !model.trim().is_empty())
            .map(|model| format!("{}/{model}", provider.name))
            .unwrap_or_else(|| primary.into())
    }

    /// Synchronous configuration selection shared by factories and builders.
    /// A missing declared default is an error rather than an invented model ID.
    pub fn configured_default_model(
        &self,
        preferred_provider: Option<&str>,
    ) -> crate::Result<String> {
        if let Some(model) = std::env::var("CADE_DEFAULT_MODEL")
            .ok()
            .filter(|model| !model.trim().is_empty())
        {
            crate::types::validate_model_id(&model)?;
            return Ok(model);
        }
        let explicit = std::env::var("CADE_LLM_PROVIDER")
            .ok()
            .filter(|provider| !provider.trim().is_empty());
        let provider = match preferred_provider.or(explicit.as_deref()) {
            Some(name) => self.get(name).ok_or_else(|| {
                crate::Error::custom(format!(
                    "Provider '{name}' has no configured definition; specify a model explicitly"
                ))
            })?,
            None => self.detected_default().ok_or_else(|| {
                crate::Error::custom("No configured default provider; specify a model explicitly")
            })?,
        };
        self.default_model_id_for(&provider.name).ok_or_else(|| {
            crate::Error::custom(format!(
                "Provider '{}' has no configured default model; specify a model explicitly",
                provider.name
            ))
        })
    }
    pub fn new() -> Self {
        Self {
            providers: BUNDLED_PROVIDERS.clone(),
        }
    }

    pub fn load_or_default(path: Option<&std::path::Path>) -> Self {
        let Some(path) = path else {
            return Self::new();
        };
        match std::fs::read_to_string(path) {
            Ok(content) => match Self::from_json(&content) {
                Ok(registry) => registry,
                Err(e) => {
                    tracing::warn!("Invalid provider registry {}: {e}", path.display());
                    Self::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                use std::io::Write;
                let seed = || -> std::io::Result<()> {
                    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)?
                        .write_all(include_str!("default_providers.json").as_bytes())
                };
                if let Err(e) = seed()
                    && e.kind() != std::io::ErrorKind::AlreadyExists
                {
                    tracing::warn!("Cannot seed provider registry {}: {e}", path.display());
                }
                Self::new()
            }
            Err(e) => {
                tracing::warn!("Cannot read provider registry {}: {e}", path.display());
                Self::new()
            }
        }
    }

    /// Merge partial native overrides and arbitrary new definitions atomically.
    /// Invalid definitions return an error, never a partially applied registry.
    pub fn from_json(content: &str) -> crate::Result<Self> {
        let defaults = Self::new();
        let raw: Vec<serde_json::Value> = serde_json::from_str(content)?;
        let mut providers = Vec::new();
        for record in raw {
            let overrides = record
                .as_object()
                .ok_or_else(|| crate::Error::custom("Provider definition must be an object"))?;
            let name = record["name"]
                .as_str()
                .ok_or_else(|| crate::Error::custom("Provider name is required"))?;
            let bundled = defaults.providers.iter().find(|p| p.name == name);
            let mut merged = match bundled {
                Some(p) => serde_json::to_value(p)?,
                None => serde_json::json!({}),
            };
            for (key, value) in overrides {
                merged[key] = value.clone();
            }
            if let Some(bundled) = bundled
                && !overrides.contains_key("models_url")
                && merged["chat_url"].as_str() != Some(bundled.chat_url.as_str())
            {
                let kind = merged["kind"].as_str().unwrap_or("openai-compatible");
                let url = merged["chat_url"]
                    .as_str()
                    .and_then(|url| crate::discovery::models_endpoint(kind, url));
                merged["models_url"] = serde_json::to_value(url)?;
            }
            providers.push(serde_json::from_value::<ProviderDef>(merged)?);
        }
        for bundled in defaults.providers {
            if !providers.iter().any(|p| p.name == bundled.name) {
                providers.push(bundled);
            }
        }
        let mut names = BTreeSet::new();
        for provider in &providers {
            if !valid_provider_name(&provider.name) || !names.insert(provider.name.clone()) {
                return Err(crate::Error::custom("Invalid or duplicate provider name"));
            }
            if !matches!(
                provider.kind.as_str(),
                "anthropic" | "openai" | "gemini" | "google" | "ollama" | "openai-compatible"
            ) {
                return Err(crate::Error::custom("Unsupported provider protocol kind"));
            }
            for endpoint in std::iter::once(&provider.chat_url).chain(provider.models_url.iter()) {
                let url = reqwest::Url::parse(endpoint).map_err(crate::Error::custom_from_err)?;
                if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                    return Err(crate::Error::custom(
                        "Provider endpoint must be HTTP(S) with a host",
                    ));
                }
            }
            if provider
                .env_vars
                .iter()
                .chain(&provider.base_url_env)
                .any(|v| !valid_env_name(v))
            {
                return Err(crate::Error::custom(
                    "Invalid provider environment variable name",
                ));
            }
            for model in provider
                .default_model
                .iter()
                .chain(&provider.fast_model)
                .chain(&provider.background_model)
            {
                crate::types::validate_model_id(model)?;
            }
            for (name, value) in &provider.headers {
                reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(crate::Error::custom_from_err)?;
                reqwest::header::HeaderValue::from_str(value)
                    .map_err(crate::Error::custom_from_err)?;
            }
        }
        let mut aliases = BTreeMap::new();
        for provider in &providers {
            for alias in &provider.aliases {
                if !valid_provider_name(alias) {
                    return Err(crate::Error::custom("Invalid provider alias"));
                }
                if !names.contains(alias)
                    && let Some(owner) = aliases.insert(alias.clone(), provider.name.clone())
                    && owner != provider.name
                {
                    return Err(crate::Error::custom("Ambiguous provider alias"));
                }
            }
        }
        Ok(Self { providers })
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

    #[test]
    fn legacy_provider_override_inherits_native_kind_defaults_and_gateway_discovery() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("providers.json");
        std::fs::write(&path, r#"[
            {"name":"openai","env_vars":["PRIVATE_KEY"],"chat_url":"http://127.0.0.1:1234/mounted/chat/completions"},
            {"name":"unfamiliar-gateway","chat_url":"http://127.0.0.1:1235/v1","default_model":"tenant/custom"}
        ]"#).unwrap();
        let registry = ProviderRegistry::load_or_default(Some(&path));
        let native = registry.get("openai").unwrap();
        assert_eq!(native.kind, "openai");
        assert_eq!(native.env_vars, ["PRIVATE_KEY"]);
        assert_eq!(native.config_key.as_deref(), Some("openai"));
        assert_eq!(native.default_model.as_deref(), Some("gpt-4o"));
        assert_eq!(
            native.models_url.as_deref(),
            Some("http://127.0.0.1:1234/mounted/models")
        );
        let custom = registry.get("unfamiliar-gateway").unwrap();
        assert_eq!(custom.kind, "openai-compatible");
        assert!(custom.discovery);
        assert_eq!(custom.default_model.as_deref(), Some("tenant/custom"));
    }
}
