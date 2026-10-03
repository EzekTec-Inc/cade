use crate::provider_registry::{ProviderDef, ProviderRegistry};
use crate::runtime::{RegisteredModel, RuntimeRegistry};
use crate::*;
use std::{collections::HashMap, sync::Arc};

pub type SharedModelRegistry = Arc<parking_lot::RwLock<RuntimeRegistry>>;

pub fn openai_compat_presets() -> Vec<(String, String)> {
    ProviderRegistry::configured()
        .get_all_providers()
        .iter()
        .filter(|p| p.kind == "openai-compatible")
        .map(|p| (p.name.clone(), p.endpoint()))
        .collect()
}

/// Cloneable routing snapshot: provider Arcs survive replacement/disconnection,
/// and no router lock needs to cover network I/O or stream consumption.
#[derive(Clone)]
pub struct LlmRouter {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    default_provider: String,
    definitions: ProviderRegistry,
    pub models: SharedModelRegistry,
    pub ollama_base_url: String,
}

impl LlmRouter {
    /// Embedded/manual registration without environment credential discovery.
    pub fn empty(default_provider: String, models: SharedModelRegistry) -> Self {
        Self {
            providers: HashMap::new(),
            default_provider,
            definitions: ProviderRegistry::new(),
            models,
            ollama_base_url: String::new(),
        }
    }

    pub fn with_provider_registry(mut self, definitions: ProviderRegistry) -> Self {
        self.models.write().set_provider_registry(&definitions);
        self.definitions = definitions;
        self
    }

    pub fn build(config: &AiConfig) -> Self {
        Self::build_with_registry(config, crate::runtime::shared_registry())
    }

    pub fn build_with_registry(config: &AiConfig, models: SharedModelRegistry) -> Self {
        let mut router = Self {
            providers: HashMap::new(),
            default_provider: config.llm_provider.clone(),
            definitions: ProviderRegistry::configured(),
            models,
            ollama_base_url: config.ollama_base_url.clone(),
        };
        router
            .models
            .write()
            .set_provider_registry(&router.definitions);
        let definitions = router.definitions.get_all_providers().to_vec();
        for def in definitions {
            let configured_key = match def.config_key.as_deref() {
                Some("anthropic") => config.anthropic_api_key.clone(),
                Some("openai") => config.openai_api_key.clone(),
                Some("google") => config.google_api_key.clone(),
                Some("deepseek") => config.deepseek_api_key.clone(),
                _ => None,
            };
            if let Some(key) = configured_key
                .or_else(|| def.env_key())
                .or_else(|| def.local.then(String::new))
            {
                let endpoint = if def.config_key.as_deref() == Some("ollama") {
                    config.ollama_base_url.clone()
                } else {
                    def.endpoint()
                };
                router.add_configured_provider(
                    &def.name,
                    &def.kind,
                    Some(key),
                    Some(endpoint),
                    config,
                );
            }
        }
        router.default_provider = config.llm_provider.clone();
        // Config names may be aliases. Never choose a randomized HashMap iteration default.
        if let Some(def) = router.definitions.get(&router.default_provider) {
            router.default_provider = def.name.clone();
        }
        router.ensure_default();
        router
    }

    fn ensure_default(&mut self) {
        if !self.providers.contains_key(&self.default_provider) {
            self.default_provider = self.provider_names().into_iter().next().unwrap_or_default();
        }
    }

    pub fn add_provider(&mut self, name: String, provider: Arc<dyn LlmProvider>) {
        if let Some(definition) = self.definitions.get(&name).cloned() {
            self.providers
                .insert(definition.name.clone(), Arc::clone(&provider));
            for alias in definition.aliases {
                if self
                    .definitions
                    .get(&alias)
                    .is_some_and(|owner| owner.name == definition.name)
                {
                    self.providers.insert(alias, Arc::clone(&provider));
                }
            }
        }
        self.providers.insert(name, provider);
        self.ensure_default();
    }

    /// Compatibility registration. Prefer add_configured_provider for endpoints
    /// loaded from DB so discovery uses the same gateway as execution.
    pub fn add_provider_with_key(
        &mut self,
        name: String,
        provider: Arc<dyn LlmProvider>,
        key: String,
    ) {
        if let Some(def) = self.definitions.get(&name).cloned() {
            let registered = RegisteredProvider::new(
                provider,
                def.kind.clone(),
                if def.discovery {
                    def.models_url
                        .clone()
                        .or_else(|| crate::discovery::models_endpoint(&def.kind, &def.endpoint()))
                } else {
                    None
                },
                key,
                name.clone(),
                Arc::clone(&self.models),
            )
            .with_headers(def.headers.clone());
            self.add_provider(name, Arc::new(registered));
        } else {
            self.add_provider(name, provider);
        }
    }

    #[cfg(feature = "rig-compat")]
    pub fn add_rig_provider<M: rig::completion::CompletionModel + Clone + Send + Sync + 'static>(
        &mut self,
        name: String,
        model: M,
    ) {
        self.add_provider(
            name,
            Arc::new(crate::rig_adapter::RigProviderAdapter { model }),
        );
    }

    pub fn register_model(&mut self, model: RegisteredModel) {
        self.models.write().register(model);
    }

    /// Canonical endpoint-aware registration used by the server and embedded SDK.
    pub fn add_configured_provider(
        &mut self,
        name: &str,
        kind: &str,
        api_key: Option<String>,
        base_url: Option<String>,
        config: &AiConfig,
    ) -> bool {
        if !crate::provider_registry::valid_provider_name(name) {
            return false;
        }
        let def = self.definitions.get(name).cloned();
        let canonical_name = def.as_ref().map(|d| d.name.as_str()).unwrap_or(name);
        let endpoint = base_url
            .filter(|url| !url.trim().is_empty())
            .or_else(|| def.as_ref().map(ProviderDef::endpoint))
            .or_else(|| self.definitions.get(kind).map(ProviderDef::endpoint));
        let api_key = api_key.or_else(|| def.as_ref().and_then(ProviderDef::env_key));
        let Some(provider) = Self::provider_with_registry(
            kind,
            api_key.clone(),
            endpoint.clone(),
            config,
            canonical_name,
            Arc::clone(&self.models),
            def.as_ref(),
        ) else {
            return false;
        };
        let models_url = if def.as_ref().is_some_and(|d| !d.discovery) {
            None
        } else if def
            .as_ref()
            .is_some_and(|d| endpoint.as_ref().is_none_or(|e| e == &d.chat_url))
        {
            def.as_ref().and_then(|d| d.models_url.clone()).or_else(|| {
                endpoint
                    .as_deref()
                    .and_then(|url| crate::discovery::models_endpoint(kind, url))
            })
        } else {
            endpoint
                .as_deref()
                .and_then(|url| crate::discovery::models_endpoint(kind, url))
        };
        let key = api_key.unwrap_or_else(|| match kind {
            "anthropic" => config.anthropic_api_key.clone().unwrap_or_default(),
            "openai" => config.openai_api_key.clone().unwrap_or_default(),
            "gemini" => config.google_api_key.clone().unwrap_or_default(),
            _ => String::new(),
        });
        let registered: Arc<dyn LlmProvider> = Arc::new(
            RegisteredProvider::new(
                provider,
                kind.into(),
                models_url,
                key,
                canonical_name.into(),
                Arc::clone(&self.models),
            )
            .with_headers(def.as_ref().map(|d| d.headers.clone()).unwrap_or_default()),
        );
        self.add_provider(name.into(), registered);
        if name == config.llm_provider || canonical_name == config.llm_provider {
            self.default_provider = canonical_name.into();
        }
        self.ensure_default();
        true
    }

    pub fn hot_sync_env_providers(&mut self) {
        self.definitions = ProviderRegistry::configured();
        self.models.write().set_provider_registry(&self.definitions);
        let config = AiConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            deepseek_api_key: None,
            ollama_base_url: self.ollama_base_url.clone(),
            llm_provider: self.default_provider.clone(),
        };
        for def in self.definitions.get_all_providers().to_vec() {
            // Explicit DB/SDK registration wins over environment rescan.
            if !self.providers.contains_key(&def.name)
                && let Some(key) = def.env_key()
            {
                self.add_configured_provider(
                    &def.name,
                    &def.kind,
                    Some(key),
                    Some(def.endpoint()),
                    &config,
                );
            }
        }
    }

    pub fn remove_provider(&mut self, name: &str) -> bool {
        let canonical = self
            .definitions
            .get(name)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| name.into());
        let Some(removed) = self
            .providers
            .get(&canonical)
            .or_else(|| self.providers.get(name))
            .cloned()
        else {
            return false;
        };
        self.providers.retain(|_, p| !Arc::ptr_eq(p, &removed));
        self.ensure_default();
        true
    }

    pub fn provider_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.providers.keys().cloned().collect();
        names.sort();
        names
    }

    pub async fn list_dynamic_models(&self) -> Vec<ModelEntry> {
        let mut seen = Vec::<Arc<dyn LlmProvider>>::new();
        let mut tasks = Vec::new();
        for name in self.provider_names() {
            if self
                .definitions
                .get(&name)
                .is_some_and(|d| d.name != name && self.providers.contains_key(&d.name))
            {
                continue;
            }
            let provider = Arc::clone(&self.providers[&name]);
            if seen.iter().any(|p| Arc::ptr_eq(p, &provider)) {
                continue;
            }
            seen.push(Arc::clone(&provider));
            let models = Arc::clone(&self.models);
            tasks.push(async move {
                match provider.discover_models().await {
                    Ok(live) if !live.is_empty() => live,
                    Ok(_) => models.read().offline_models(&name),
                    Err(e) => {
                        tracing::warn!("Model discovery for {name}: {e}");
                        models.read().offline_models(&name)
                    }
                }
            });
        }
        let mut entries: Vec<_> = futures::future::join_all(tasks)
            .await
            .into_iter()
            .flatten()
            .collect();
        entries.sort_by(|a, b| a.provider.cmp(&b.provider).then(a.id.cmp(&b.id)));
        entries.dedup_by(|a, b| a.id == b.id);
        entries
    }

    pub fn provider_from_row(
        kind: &str,
        api_key: Option<String>,
        base_url: Option<String>,
        config: &AiConfig,
    ) -> Option<Arc<dyn LlmProvider>> {
        Self::provider_with_registry(
            kind,
            api_key,
            base_url,
            config,
            kind,
            crate::runtime::shared_registry(),
            None,
        )
    }

    fn provider_with_registry(
        kind: &str,
        api_key: Option<String>,
        base_url: Option<String>,
        config: &AiConfig,
        name: &str,
        models: SharedModelRegistry,
        definition: Option<&ProviderDef>,
    ) -> Option<Arc<dyn LlmProvider>> {
        match kind {
            "anthropic" => {
                let mut provider = anthropic::AnthropicProvider::new(
                    api_key.or_else(|| config.anthropic_api_key.clone())?,
                    base_url,
                )
                .with_registry(name.into(), models);
                if let Some(definition) = definition {
                    provider = provider.with_provider_definition(definition);
                }
                Some(Arc::new(provider))
            }
            "openai" | "openai-compatible" => {
                let key = if kind == "openai" {
                    api_key.or_else(|| config.openai_api_key.clone())?
                } else {
                    api_key.unwrap_or_default()
                };
                let url = if kind == "openai-compatible" {
                    Some(
                        base_url
                            .filter(|s| !s.trim().is_empty())
                            .or_else(|| std::env::var("OPENAI_COMPATIBLE_BASE_URL").ok())?,
                    )
                } else {
                    base_url
                };
                let mut provider =
                    openai::OpenAiProvider::new(key, url).with_registry(name.into(), models);
                if let Some(definition) = definition {
                    provider = provider.with_provider_definition(definition);
                }
                Some(Arc::new(provider))
            }
            "gemini" | "google" => {
                let mut provider = gemini::GeminiProvider::new(
                    api_key.or_else(|| config.google_api_key.clone())?,
                    base_url,
                )
                .with_registry(name.into(), models);
                if let Some(definition) = definition {
                    provider = provider.with_provider_definition(definition);
                }
                Some(Arc::new(provider))
            }
            "ollama" => {
                let mut provider = ollama::OllamaProvider::new(
                    base_url
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| config.ollama_base_url.clone()),
                )
                .with_registry(name.into(), models);
                if let Some(definition) = definition {
                    provider = provider.with_provider_definition(definition);
                }
                Some(Arc::new(provider))
            }
            _ => None,
        }
    }

    pub fn resolve_provider_name(&self, model: &str) -> Result<(String, String)> {
        crate::types::validate_model_id(model)?;
        let models = self.models.read();
        let model = models.registered_id(model);
        if let Some((prefix, bare)) = model.split_once('/') {
            if self.providers.contains_key(prefix) {
                return Ok((prefix.into(), bare.into()));
            }
            return Err(Error::custom(format!(
                "Provider '{prefix}' is not configured. Run /connect {prefix} to add it."
            )));
        }
        let candidates = models.candidates(model);
        if !candidates.is_empty() {
            return candidates
                .iter()
                .find(|p| self.providers.contains_key(*p))
                .map(|p| (p.clone(), model.into()))
                .ok_or_else(|| {
                    Error::custom(format!(
                        "Model '{model}' requires a configured provider: {}",
                        candidates.join(", ")
                    ))
                });
        }
        if self.providers.contains_key(&self.default_provider) {
            return Ok((self.default_provider.clone(), model.into()));
        }
        Err(Error::custom("No LLM provider available"))
    }

    pub fn resolve_provider(&self, model: &str) -> Result<(Arc<dyn LlmProvider>, String)> {
        let (name, bare) = self.resolve_provider_name(model)?;
        let provider = Arc::clone(&self.providers[&name]);
        provider.validate_model(&bare)?;
        Ok((provider, bare))
    }

    pub fn validate_model(&self, model: &str) -> Result<()> {
        self.resolve_provider(model).map(|_| ())
    }

    /// Exact normalized compatibility mapping; never strip or rewrite arbitrary native IDs.
    pub fn map_openrouter_to_native(&self, bare: &str) -> Option<(String, String)> {
        let (provider, model) = bare.split_once('/')?;
        let provider = self
            .definitions
            .get(provider)
            .map(|d| d.name.as_str())
            .unwrap_or(provider);
        if !self.providers.contains_key(provider) {
            return None;
        }
        let clean = model.split(':').next().unwrap_or(model);
        let norm = |s: &str| s.to_lowercase().replace('.', "-");
        let models = self.models.read();
        let exact = models.offline_models(provider).into_iter().find(|m| {
            m.id.split_once('/')
                .is_some_and(|(_, id)| norm(id) == norm(clean))
        });
        Some((
            provider.into(),
            exact
                .and_then(|m| m.id.split_once('/').map(|(_, id)| id.to_owned()))
                .unwrap_or_else(|| clean.into()),
        ))
    }

    fn route(&self, req: &CompletionRequest) -> Result<Route> {
        let (name, bare) = self.resolve_provider_name(&req.model)?;
        let provider = Arc::clone(&self.providers[&name]);
        provider.validate_model(&bare)?;
        let may_failover = self.models.read().failover_providers.contains(&name);
        let fallback = if may_failover {
            self.map_openrouter_to_native(&bare)
                .and_then(|(name, model)| {
                    self.providers
                        .get(&name)
                        .map(|p| (Arc::clone(p), model.clone(), format!("{name}/{model}")))
                })
        } else {
            None
        };
        Ok(Route {
            provider,
            usage_model: format!("{name}/{bare}"),
            request: CompletionRequest {
                model: bare,
                ..req.clone()
            },
            fallback,
        })
    }

    /// Proactively probe candidate cheaper models with a lightweight ping completion
    /// to ensure they work before switching. Cascades through alternatives until success.
    pub async fn probe_and_route_cheapest_verified(
        &self,
        req: &CompletionRequest,
        candidate_models: &[String],
    ) -> Result<(Arc<dyn LlmProvider>, CompletionRequest, String)> {
        let probe_req = CompletionRequest {
            model: "".to_string(),
            messages: vec![crate::LlmMessage {
                role: "user".to_string(),
                content: "ping".to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            }],
            tools: Vec::new(),
            max_tokens: 5,
            reasoning_effort: None,
        };

        for candidate in candidate_models {
            let (provider, bare_model) = match self.resolve_provider(candidate) {
                Ok(res) => res,
                Err(_) => continue,
            };

            let candidate_req = CompletionRequest {
                model: bare_model.clone(),
                ..probe_req.clone()
            };

            tracing::debug!("Probing candidate cheaper model: {candidate}");
            match provider.complete(&candidate_req).await {
                Ok(_) => {
                    tracing::info!("Health check succeeded for cheaper model: {candidate}");
                    let target_req = CompletionRequest {
                        model: bare_model,
                        ..req.clone()
                    };
                    let usage_model = candidate.to_string();
                    return Ok((provider, target_req, usage_model));
                }
                Err(e) => {
                    tracing::warn!("Health check failed for cheaper model {candidate}: {e}; trying next alternative");
                }
            }
        }

        // Fallback to original route if all probes fail
        let route = self.route(req)?;
        Ok((route.provider, route.request, route.usage_model))
    }
}

struct Route {
    provider: Arc<dyn LlmProvider>,
    request: CompletionRequest,
    usage_model: String,
    fallback: Option<(Arc<dyn LlmProvider>, String, String)>,
}

impl Route {
    fn failover(&self, error: &Error) -> Option<(Arc<dyn LlmProvider>, CompletionRequest, String)> {
        // Only upstream connection/status failures qualify; parsing/schema errors do not.
        let eligible = match error {
            Error::Provider { .. } => true,
            Error::Reqwest(error) => error.is_connect() || error.is_timeout() || error.is_request(),
            _ => false,
        };
        if !eligible {
            return None;
        }
        let (provider, model, usage_model) = self.fallback.as_ref()?;
        if provider.validate_model(model).is_err() {
            return None;
        }
        tracing::warn!("Upstream failed: {error}; trying configured native route {model}");
        Some((
            Arc::clone(provider),
            CompletionRequest {
                model: model.clone(),
                ..self.request.clone()
            },
            usage_model.clone(),
        ))
    }
}

#[async_trait::async_trait]
impl LlmProvider for LlmRouter {
    fn default_model(&self) -> Option<String> {
        if !self.providers.contains_key(&self.default_provider) {
            return None;
        }
        self.definitions
            .default_model_id_for(&self.default_provider)
            .or_else(|| {
                self.models
                    .read()
                    .offline_models(&self.default_provider)
                    .first()
                    .map(|model| model.id.clone())
            })
    }
    async fn discover_models(&self) -> Result<Vec<ModelEntry>> {
        Ok(self.list_dynamic_models().await)
    }
    fn validate_model(&self, model: &str) -> Result<()> {
        LlmRouter::validate_model(self, model)
    }
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let route = self.route(req)?;
        match route.provider.complete(&route.request).await {
            Ok(result) => Ok(result),
            Err(e) => match route.failover(&e) {
                Some((p, req, _)) => p.complete(&req).await,
                None => Err(e),
            },
        }
    }
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk>> + Send>>> {
        let route = self.route(req)?;
        match route.provider.stream(&route.request).await {
            Ok(stream) => Ok(tag_stream_usage(stream, route.usage_model.clone())),
            Err(e) => match route.failover(&e) {
                Some((p, req, model)) => Ok(tag_stream_usage(p.stream(&req).await?, model)),
                None => Err(e),
            },
        }
    }
    async fn complete_structured(
        &self,
        req: &CompletionRequest,
        schema: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let route = self.route(req)?;
        match route
            .provider
            .complete_structured(&route.request, schema.clone())
            .await
        {
            Ok(result) => Ok(result),
            Err(e) => match route.failover(&e) {
                Some((p, req, _)) => p.complete_structured(&req, schema).await,
                None => Err(e),
            },
        }
    }
}

fn tag_stream_usage(
    stream: std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk>> + Send>>,
    model: String,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk>> + Send>> {
    use futures::StreamExt;
    Box::pin(stream.map(move |chunk| {
        chunk.map(|chunk| match chunk {
            StreamChunk::Usage(mut usage) => {
                usage.model = model.clone();
                StreamChunk::Usage(usage)
            }
            other => other,
        })
    }))
}

/// Shared adapter used by server and SDK. Sync validation uses try_read and reports
/// a transient busy error instead of silently accepting an unvalidated model.
pub struct ConcurrentRouter(pub Arc<tokio::sync::RwLock<LlmRouter>>);

#[async_trait::async_trait]
impl LlmProvider for ConcurrentRouter {
    fn default_model(&self) -> Option<String> {
        self.0
            .try_read()
            .ok()
            .and_then(|router| LlmProvider::default_model(&*router))
    }
    async fn discover_models(&self) -> Result<Vec<ModelEntry>> {
        let router = self.0.read().await.clone();
        Ok(router.list_dynamic_models().await)
    }
    fn validate_model(&self, model: &str) -> Result<()> {
        self.0
            .try_read()
            .map_err(|_| {
                Error::custom("Provider configuration is being updated; retry model validation")
            })?
            .validate_model(model)
    }
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let router = self.0.read().await.clone();
        router.complete(req).await
    }
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk>> + Send>>> {
        let router = self.0.read().await.clone();
        router.stream(req).await
    }
    async fn complete_structured(
        &self,
        req: &CompletionRequest,
        schema: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let router = self.0.read().await.clone();
        router.complete_structured(req, schema).await
    }
}

struct RegisteredProvider {
    inner: Arc<dyn LlmProvider>,
    kind: String,
    models_url: Option<String>,
    key: String,
    name: String,
    models: SharedModelRegistry,
    headers: std::collections::BTreeMap<String, String>,
}
impl RegisteredProvider {
    fn new(
        inner: Arc<dyn LlmProvider>,
        kind: String,
        models_url: Option<String>,
        key: String,
        name: String,
        models: SharedModelRegistry,
    ) -> Self {
        Self {
            inner,
            kind,
            models_url,
            key,
            name,
            models,
            headers: std::collections::BTreeMap::new(),
        }
    }

    fn with_headers(mut self, headers: std::collections::BTreeMap<String, String>) -> Self {
        self.headers = headers;
        self
    }
}
#[async_trait::async_trait]
impl LlmProvider for RegisteredProvider {
    fn validate_model(&self, model: &str) -> Result<()> {
        self.inner.validate_model(model)
    }
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        self.inner.complete(req).await
    }
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk>> + Send>>> {
        self.inner.stream(req).await
    }
    async fn complete_structured(
        &self,
        req: &CompletionRequest,
        schema: serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.inner.complete_structured(req, schema).await
    }
    async fn discover_models(&self) -> Result<Vec<ModelEntry>> {
        let Some(url) = &self.models_url else {
            return Ok(Vec::new());
        };
        crate::discovery::discover_with_headers(
            &self.kind,
            url,
            &self.key,
            &self.name,
            &self.models,
            &self.headers,
        )
        .await
    }
}

#[cfg(test)]
pub(crate) fn infer_provider_candidates(model: &str) -> Vec<String> {
    RuntimeRegistry::default().candidates(model)
}
