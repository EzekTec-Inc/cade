//! Editable execution metadata. Rules are compatibility seeds, never model allowlists.
use crate::{
    ModelEntry,
    openai::{ApiProtocol, ReasoningStrategy, TokenParameter},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenizerKind {
    Cl100kBase,
    O200kBase,
    Characters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptCacheKind {
    Anthropic,
    Openai,
    Gemini,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSelectorKind {
    Intent,
    Needle,
    PassThrough,
}

/// Process runtime metadata shared by execution, discovery, and legacy budgeting
/// helpers. Applications needing isolation can supply their own SharedModelRegistry.
pub fn shared_registry() -> crate::SharedModelRegistry {
    static REGISTRY: std::sync::LazyLock<crate::SharedModelRegistry> =
        std::sync::LazyLock::new(|| {
            std::sync::Arc::new(parking_lot::RwLock::new(RuntimeRegistry::configured()))
        });
    std::sync::Arc::clone(&REGISTRY)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelMetadata {
    pub display_name: Option<String>,
    pub toolset: Option<String>,
    pub max_tokens: Option<u32>,
    pub context_window: Option<u32>,
    /// None means unknown; explicit false rejects requests requiring tools.
    pub tools: Option<bool>,
    pub native_structured: Option<bool>,
    pub protocol: Option<ApiProtocol>,
    pub token_parameter: Option<TokenParameter>,
    pub reasoning: Option<ReasoningStrategy>,
    pub developer_role: Option<bool>,
    pub preview_gateway: Option<bool>,
    pub max_tools: Option<usize>,
    /// Provider wire spelling, e.g. adaptive, budget, level, deepseek.
    pub thinking: Option<String>,
    pub thinking_budgets: Option<BTreeMap<String, i64>>,
    pub reasoning_values: Option<BTreeMap<String, String>>,
    pub include_reasoning: Option<bool>,
    pub tokenizer: Option<TokenizerKind>,
    pub chars_per_token: Option<usize>,
    pub prompt_cache: Option<PromptCacheKind>,
    pub snapshot_aliases: Option<bool>,
    pub tool_selector: Option<ToolSelectorKind>,
    pub cache_token_boundary: Option<usize>,
    pub cache_character_boundary: Option<usize>,
    pub cache_padding_limit: Option<usize>,
    pub prefer_primary_for_background: Option<bool>,
}

impl ModelMetadata {
    /// Emergency one-character estimation is conservative even for incomplete
    /// programmatic metadata. Valid configured ratios come from JSON.
    pub fn character_ratio(&self) -> usize {
        self.chars_per_token.unwrap_or(1).max(1)
    }

    pub(crate) fn overlay(&mut self, other: &Self) {
        macro_rules! merge { ($($field:ident),*) => { $(if other.$field.is_some() { self.$field = other.$field.clone(); })* }; }
        merge!(
            display_name,
            toolset,
            max_tokens,
            context_window,
            tools,
            native_structured,
            protocol,
            token_parameter,
            reasoning,
            developer_role,
            preview_gateway,
            max_tools,
            thinking,
            thinking_budgets,
            reasoning_values,
            include_reasoning,
            tokenizer,
            chars_per_token,
            prompt_cache,
            snapshot_aliases,
            tool_selector,
            cache_token_boundary,
            cache_character_boundary,
            cache_padding_limit,
            prefer_primary_for_background
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredModel {
    /// Fully qualified routing ID; nested upstream IDs are preserved verbatim.
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(flatten)]
    pub metadata: ModelMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityRule {
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default)]
    pub prefixes: Vec<String>,
    #[serde(default)]
    pub contains_any: Vec<String>,
    #[serde(default)]
    pub suffixes: Vec<String>,
    #[serde(flatten)]
    pub metadata: ModelMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRule {
    pub prefixes: Vec<String>,
    pub providers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeRegistry {
    #[serde(default)]
    pub fallback: ModelMetadata,
    #[serde(default)]
    pub rules: Vec<CompatibilityRule>,
    #[serde(default)]
    pub models: Vec<RegisteredModel>,
    #[serde(default)]
    pub seeded_models: Vec<RegisteredModel>,
    #[serde(default)]
    pub routing: Vec<RoutingRule>,
    #[serde(default)]
    pub failover_providers: Vec<String>,
    /// Live observations are separate from explicit overrides, so a refresh can
    /// update provider limits without overwriting editable registrations.
    #[serde(skip)]
    pub discovered: BTreeMap<String, ModelMetadata>,
    #[serde(skip)]
    pub provider_aliases: BTreeMap<String, String>,
}

impl Default for RuntimeRegistry {
    fn default() -> Self {
        static BUNDLED: std::sync::LazyLock<RuntimeRegistry> = std::sync::LazyLock::new(|| {
            match serde_json::from_str::<RuntimeRegistry>(include_str!("default_models.json")) {
                Ok(mut registry) => {
                    registry
                        .set_provider_registry(&crate::provider_registry::ProviderRegistry::new());
                    if let Err(e) = registry.validate() {
                        tracing::error!("Invalid bundled model registry: {e}");
                        return RuntimeRegistry::empty();
                    }
                    registry
                }
                Err(e) => {
                    tracing::error!("Invalid bundled model registry: {e}");
                    RuntimeRegistry::empty()
                }
            }
        });
        BUNDLED.clone()
    }
}

pub fn config_path(env: &str, file: &str) -> Option<std::path::PathBuf> {
    crate::provider_registry::valid_env_name(env)
        .then(|| std::env::var_os(env))
        .flatten()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| std::path::PathBuf::from(home).join(".cade").join(file))
        })
}

impl RuntimeRegistry {
    pub(crate) fn upstream_model<'a>(&self, provider: &str, model: &'a str) -> &'a str {
        let full = format!("{provider}/{model}");
        // A registered/discovered upstream ID can itself begin with the routing
        // provider's name; preserve it instead of removing a second prefix.
        if self.models.iter().any(|m| m.id == full) || self.discovered.contains_key(&full) {
            return model;
        }
        model.strip_prefix(&format!("{provider}/")).unwrap_or(model)
    }

    pub fn configured() -> Self {
        let providers = crate::provider_registry::ProviderRegistry::configured();
        let mut registry = Self::load(config_path("CADE_MODELS_CONFIG", "models.json").as_deref());
        registry.set_provider_registry(&providers);
        registry
    }

    /// Custom rules precede bundled rules; exact custom registrations override seeds.
    /// Missing fallback fields inherit bundled conservative budgeting defaults.
    pub fn load(path: Option<&std::path::Path>) -> Self {
        let mut registry = Self::default();
        if let Some(path) = path {
            match std::fs::read_to_string(path) {
                Ok(text) => match Self::merge_json(&text, registry.clone()) {
                    Ok(custom) => registry = custom,
                    Err(e) => tracing::warn!("Invalid model registry {}: {e}", path.display()),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!("Cannot read model registry {}: {e}", path.display()),
            }
        }
        registry
    }

    pub fn from_json(text: &str) -> crate::Result<Self> {
        Self::merge_json(text, Self::default())
    }

    fn merge_json(text: &str, registry: Self) -> crate::Result<Self> {
        let raw: serde_json::Value = serde_json::from_str(text)?;
        let mut custom: Self = serde_json::from_value(raw.clone())?;
        let mut fallback = registry.fallback.clone();
        fallback.overlay(&custom.fallback);
        custom.fallback = fallback;
        custom.rules.extend(registry.rules);
        custom.routing.extend(registry.routing);
        custom.models.extend(registry.models);
        custom.seeded_models.extend(registry.seeded_models);
        if raw.get("failover_providers").is_none() {
            custom.failover_providers = registry.failover_providers;
        }
        custom.provider_aliases = registry.provider_aliases;
        custom.validate()?;
        Ok(custom)
    }

    fn empty() -> Self {
        Self {
            fallback: ModelMetadata::default(),
            rules: Vec::new(),
            models: Vec::new(),
            seeded_models: Vec::new(),
            routing: Vec::new(),
            failover_providers: Vec::new(),
            discovered: BTreeMap::new(),
            provider_aliases: BTreeMap::new(),
        }
    }

    pub fn set_provider_registry(
        &mut self,
        providers: &crate::provider_registry::ProviderRegistry,
    ) {
        self.provider_aliases = providers.alias_map();
    }

    fn validate_metadata(metadata: &ModelMetadata) -> crate::Result<()> {
        if metadata.max_tokens == Some(0)
            || metadata.context_window == Some(0)
            || metadata.chars_per_token == Some(0)
            || metadata.cache_token_boundary == Some(0)
            || metadata.cache_character_boundary == Some(0)
        {
            return Err(crate::Error::custom(
                "Configured token/context limits and character ratio must be positive",
            ));
        }
        Ok(())
    }

    fn validate_model(model: &RegisteredModel) -> crate::Result<()> {
        crate::types::validate_model_id(&model.id)?;
        let (provider, _) = model
            .id
            .split_once('/')
            .ok_or_else(|| crate::Error::custom("Registered model requires a provider prefix"))?;
        if !crate::provider_registry::valid_provider_name(provider) {
            return Err(crate::Error::custom(
                "Invalid registered model provider prefix",
            ));
        }
        for alias in &model.aliases {
            crate::types::validate_model_id(alias)?;
        }
        Self::validate_metadata(&model.metadata)
    }

    fn validate(&self) -> crate::Result<()> {
        Self::validate_metadata(&self.fallback)?;
        let mut aliases = BTreeMap::new();
        for model in self.models.iter().chain(&self.seeded_models) {
            Self::validate_model(model)?;
            for alias in &model.aliases {
                if let Some(owner) = aliases.insert(alias, &model.id)
                    && owner != &model.id
                {
                    return Err(crate::Error::custom("Ambiguous registered model alias"));
                }
            }
        }
        for rule in &self.rules {
            Self::validate_metadata(&rule.metadata)?;
        }
        for provider in self
            .failover_providers
            .iter()
            .chain(self.routing.iter().flat_map(|r| &r.providers))
            .chain(self.rules.iter().flat_map(|r| &r.providers))
        {
            if !crate::provider_registry::valid_provider_name(provider) {
                return Err(crate::Error::custom("Invalid model registry provider name"));
            }
        }
        Ok(())
    }

    pub fn register(&mut self, model: RegisteredModel) {
        if let Err(e) = self.try_register(model) {
            tracing::warn!("Invalid model registration: {e}");
        }
    }

    pub fn try_register(&mut self, model: RegisteredModel) -> crate::Result<()> {
        Self::validate_model(&model)?;
        for existing in &self.models {
            if existing.id != model.id
                && model
                    .aliases
                    .iter()
                    .any(|alias| existing.aliases.contains(alias))
            {
                return Err(crate::Error::custom("Ambiguous registered model alias"));
            }
        }
        self.models.retain(|existing| existing.id != model.id);
        self.models.insert(0, model);
        Ok(())
    }

    pub fn registered_id<'a>(&'a self, id: &'a str) -> &'a str {
        self.models
            .iter()
            .find(|model| model.id == id)
            .or_else(|| {
                self.models
                    .iter()
                    .find(|model| model.aliases.iter().any(|a| a == id))
            })
            .map(|model| model.id.as_str())
            .unwrap_or(id)
    }

    /// Canonical lookup identity. Exact registrations/observations win before
    /// compatibility aliases, so numeric suffixes in custom deployment IDs survive.
    pub fn canonical_model_id(&self, id: &str) -> String {
        let registered = self.registered_id(id);
        if registered != id || self.models.iter().any(|m| m.id == id) {
            return registered.into();
        }
        let mut id = id;
        while let Some((provider, inner)) = id.split_once('/') {
            if !self
                .failover_providers
                .iter()
                .any(|p| self.canonical_provider(p) == self.canonical_provider(provider))
                || !inner.contains('/')
            {
                break;
            }
            if self.discovered.contains_key(id)
                || self.seeded_models.iter().any(|m| self.same_id(&m.id, id))
            {
                break;
            }
            id = inner;
        }
        let full = if let Some((provider, model)) = id.split_once('/') {
            format!("{}/{model}", self.canonical_provider(provider))
        } else if let Some(provider) = self.candidates(id).first() {
            format!("{}/{id}", self.canonical_provider(provider))
        } else {
            return id.into();
        };
        if self
            .models
            .iter()
            .chain(&self.seeded_models)
            .any(|m| self.same_id(&m.id, &full))
            || self.discovered.keys().any(|id| self.same_id(id, &full))
        {
            return full;
        }
        let base = crate::catalogue::strip_model_snapshot_suffix(&full);
        if base != full
            && let Some((provider, model)) = base.split_once('/')
        {
            let metadata = self.metadata(provider, model);
            if metadata.snapshot_aliases == Some(true) {
                return base.into();
            }
        }
        full
    }

    pub fn metadata_for_id(&self, id: &str) -> ModelMetadata {
        let id = self.canonical_model_id(id);
        let (provider, model) = id.split_once('/').unwrap_or(("", &id));
        self.metadata(provider, model)
    }

    /// Fresh union of editable records and discovery; owned rows cannot hold a
    /// registry guard while a caller subsequently queries metadata or tokenizers.
    pub fn catalogue_snapshot(&self) -> Vec<crate::catalogue::CatalogueRow> {
        let ids: std::collections::BTreeSet<_> = self
            .models
            .iter()
            .chain(&self.seeded_models)
            .map(|m| m.id.clone())
            .chain(self.discovered.keys().cloned())
            .collect();
        ids.into_iter()
            .filter_map(|id| {
                let (provider, model) = id.split_once('/')?;
                let entry = self.entry(provider, model, self.discovered.contains_key(&id));
                Some((
                    entry.provider,
                    entry.display_name,
                    entry.id,
                    entry.toolset,
                    entry.max_tokens,
                    entry.context_window,
                ))
            })
            .collect()
    }

    pub fn candidates(&self, bare: &str) -> Vec<String> {
        let exact: Vec<String> = self
            .models
            .iter()
            .filter(|m| {
                m.id.split_once('/').is_some_and(|(_, id)| id == bare)
                    || m.aliases.iter().any(|a| a == bare)
            })
            .filter_map(|m| m.id.split_once('/').map(|(p, _)| p.to_owned()))
            .collect();
        if !exact.is_empty() {
            return exact;
        }
        let lower = bare.to_ascii_lowercase();
        let routed = self
            .routing
            .iter()
            .find(|r| r.prefixes.iter().any(|p| lower.starts_with(p)))
            .map(|r| r.providers.clone())
            .unwrap_or_default();
        if !routed.is_empty() {
            return routed;
        }
        self.discovered
            .keys()
            .filter_map(|id| {
                let (provider, model) = id.split_once('/')?;
                (model == bare).then(|| provider.to_owned())
            })
            .collect()
    }

    pub fn metadata(&self, provider: &str, model: &str) -> ModelMetadata {
        let mut result = self.fallback.clone();
        result.overlay(&self.specific_metadata(provider, model));
        Self::reconcile_tools(&self.fallback, &mut result);
        result
    }

    fn same_id(&self, a: &str, b: &str) -> bool {
        if a == b {
            return true;
        }
        match (a.split_once('/'), b.split_once('/')) {
            (Some((pa, ma)), Some((pb, mb))) => {
                ma == mb && self.canonical_provider(pa) == self.canonical_provider(pb)
            }
            _ => false,
        }
    }

    fn canonical_provider<'a>(&'a self, provider: &'a str) -> &'a str {
        self.provider_aliases
            .get(provider)
            .map(String::as_str)
            .unwrap_or(provider)
    }

    fn specific_metadata(&self, provider: &str, model: &str) -> ModelMetadata {
        let full = format!("{provider}/{model}");
        let registered = self.registered_id(&full);
        let (owner, bare) = registered.split_once('/').unwrap_or((provider, model));
        let provider = self.canonical_provider(owner);
        let mut result = ModelMetadata::default();
        // Exact editable offline records are compatibility seeds, not an allowlist.
        for seed in self
            .seeded_models
            .iter()
            .rev()
            .filter(|seed| self.same_id(&seed.id, registered))
        {
            result.overlay(&seed.metadata);
        }
        // Earlier rules override fields, not entire recipes. Partial custom rules
        // can change one limit without discarding a bundled protocol/tokenizer.
        for rule in self.rules.iter().rev().filter(|r| {
            (r.providers.is_empty()
                || r.providers
                    .iter()
                    .any(|p| self.canonical_provider(p) == provider))
                && ((r.prefixes.is_empty() && r.contains_any.is_empty() && r.suffixes.is_empty())
                    || r.prefixes.iter().any(|p| bare.starts_with(p))
                    || r.contains_any.iter().any(|pattern| bare.contains(pattern))
                    || r.suffixes.iter().any(|suffix| bare.ends_with(suffix)))
        }) {
            result.overlay(&rule.metadata);
        }
        for (_, discovered) in self
            .discovered
            .iter()
            .filter(|(id, _)| self.same_id(id, registered))
        {
            result.overlay(discovered);
        }
        for model in self
            .models
            .iter()
            .rev()
            .filter(|m| self.same_id(&m.id, registered))
        {
            result.overlay(&model.metadata);
        }
        result
    }

    fn reconcile_tools(fallback: &ModelMetadata, result: &mut ModelMetadata) {
        if result.tools == Some(false) {
            result.toolset = Some("none".into());
        }
        if result
            .toolset
            .as_deref()
            .is_some_and(|s| matches!(s, "none" | "unsupported"))
        {
            if result.tools == Some(true) {
                // Explicit/discovered support can supersede an offline no-tools seed.
                result.toolset = fallback
                    .toolset
                    .clone()
                    .filter(|s| !matches!(s.as_str(), "none" | "unsupported"));
            } else {
                result.tools = Some(false);
            }
        }
    }

    pub fn entry(&self, provider: &str, bare: &str, dynamic: bool) -> ModelEntry {
        let metadata = self.metadata(provider, bare);
        ModelEntry {
            provider: provider.into(),
            id: format!("{provider}/{bare}"),
            display_name: metadata.display_name.unwrap_or_else(|| bare.into()),
            toolset: metadata.toolset.unwrap_or_else(|| "default".into()),
            max_tokens: metadata.max_tokens.unwrap_or(0),
            context_window: metadata.context_window.unwrap_or(0),
            dynamic,
        }
    }

    pub fn metadata_source(&self, provider: &str, bare: &str) -> &'static str {
        let full = format!("{provider}/{bare}");
        if self.models.iter().any(|m| self.same_id(&m.id, &full)) {
            "registered"
        } else if self.discovered.keys().any(|id| self.same_id(id, &full)) {
            "discovered"
        } else if self
            .seeded_models
            .iter()
            .any(|seed| self.same_id(&seed.id, &full))
        {
            "offline_catalogue"
        } else {
            "compatibility_or_fallback"
        }
    }

    pub fn has_known_limits(&self, provider: &str, bare: &str) -> bool {
        let metadata = self.specific_metadata(provider, bare);
        metadata.max_tokens.is_some() && metadata.context_window.is_some()
    }

    pub fn offline_models(&self, provider: &str) -> Vec<ModelEntry> {
        let mut entries: Vec<_> = self
            .models
            .iter()
            .filter_map(|m| {
                let (p, bare) = m.id.split_once('/')?;
                (p == provider).then(|| self.entry(provider, bare, false))
            })
            .collect();
        for seed in &self.seeded_models {
            if let Some((p, bare)) = seed.id.split_once('/')
                && p == provider
                && !entries.iter().any(|m| m.id == seed.id)
            {
                entries.push(self.entry(provider, bare, false));
            }
        }
        for id in self.discovered.keys() {
            if let Some((p, bare)) = id.split_once('/')
                && p == provider
                && !entries.iter().any(|m| m.id == *id)
            {
                entries.push(self.entry(provider, bare, false));
            }
        }
        entries
    }
}
