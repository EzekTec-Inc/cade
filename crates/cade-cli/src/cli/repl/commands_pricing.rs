//! /pricing command handler.

use crate::Result;
use super::Repl;

impl Repl {
    pub(crate) async fn cmd_pricing(
        &mut self,
        arg: Option<String>,
    ) -> Result<bool> {
        match arg.as_deref() {
            Some("sync") => {
                self.tui_dim("  Fetching live pricing from OpenRouter…");
                match fetch_openrouter_pricing().await {
                    Ok(rules) => {
                        let count = rules.len();
                        if let Some(p) = dirs::home_dir()
                            .map(|h| h.join(".cade").join("pricing.json"))
                        {
                            if let Some(parent) = p.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            match serde_json::to_string_pretty(&rules) {
                                Ok(json) => {
                                    if let Err(e) = std::fs::write(&p, &json) {
                                        self.tui_err(format!(
                                            "  Failed to write pricing.json: {e}"
                                        ));
                                    } else {
                                        let mut stats = self.session_stats.lock();
                                        stats.registry = std::sync::Arc::new(
                                            cade_ai::ModelRegistry::load_or_default(Some(&p)),
                                        );
                                        self.tui_ok(format!(
                                            "  ✓ Synced {count} models from OpenRouter → ~/.cade/pricing.json"
                                        ));
                                    }
                                }
                                Err(e) => self.tui_err(format!("  Failed to serialize: {e}")),
                            }
                        }
                    }
                    Err(e) => self.tui_err(format!("  Failed to fetch from OpenRouter: {e}")),
                }
            }
            Some(cmd) if cmd.starts_with("set ") => {
                self.tui_err("  /pricing set is not fully implemented yet. Please edit ~/.cade/pricing.json manually.");
            }
            _ => {
                let model = self.model();
                let stats = self.session_stats.lock();
                let pricing = stats.registry.pricing_for_model(&model);
                self.tui_hdr(format!("  Pricing for model: {}", model));
                self.tui_dim(format!("  Input:       ${:.4}/1M", pricing.input));
                self.tui_dim(format!("  Output:      ${:.4}/1M", pricing.output));
                self.tui_dim(format!("  Cache Read:  ${:.4}/1M", pricing.cache_read));
                self.tui_dim(format!("  Cache Write: ${:.4}/1M", pricing.cache_write));
                self.tui_dim("  /pricing sync  — update from OpenRouter (300+ models)");
            }
        }
        Ok(false)
    }
}

// ── OpenRouter pricing fetch + transform ────────────────────────────────────

/// Cache-pricing ratios by provider (relative to input price).
/// OpenRouter doesn't expose cache pricing, so we derive it from known
/// provider billing structures.
fn cache_ratios(model_id: &str) -> (f64, f64) {
    // (cache_read_ratio, cache_write_ratio) relative to input price
    if model_id.starts_with("anthropic/") {
        (0.10, 1.25) // 10% read, 125% write
    } else if model_id.starts_with("openai/") {
        (0.50, 0.0) // 50% read, no write charge
    } else if model_id.starts_with("google/") || model_id.contains("gemini") {
        (0.25, 0.0) // 25% read, no write charge
    } else if model_id.starts_with("deepseek/") {
        (0.10, 0.0)
    } else {
        // mistralai/ and unknown providers — same conservative default
        (0.25, 0.0)
    }
}

/// Fetch `GET https://openrouter.ai/api/v1/models` and transform into
/// CADE `PricingRule` format.
async fn fetch_openrouter_pricing() -> crate::Result<Vec<cade_ai::PricingRule>> {
    let url = "https://openrouter.ai/api/v1/models";
    let resp = reqwest::get(url).await.map_err(|e| format!("Request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()).into());
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("Parse failed: {e}"))?;
    let data = body["data"]
        .as_array()
        .ok_or("Missing 'data' array in response")?;

    let mut rules: Vec<cade_ai::PricingRule> = Vec::with_capacity(data.len());

    for model in data {
        let id = match model["id"].as_str() {
            Some(id) => id,
            None => continue,
        };
        let pricing = &model["pricing"];

        // OpenRouter prices are $/token as strings — parse and convert to $/1M
        let prompt_per_tok: f64 = pricing["prompt"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let completion_per_tok: f64 = pricing["completion"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);

        // Skip free/zero-priced models (they'd just clutter the file)
        if prompt_per_tok == 0.0 && completion_per_tok == 0.0 {
            continue;
        }

        let input_per_m = prompt_per_tok * 1_000_000.0;
        let output_per_m = completion_per_tok * 1_000_000.0;
        let (cr_ratio, cw_ratio) = cache_ratios(id);

        rules.push(cade_ai::PricingRule {
            contains_any: vec![id.to_string()],
            starts_with_any: Vec::new(),
            not_contains_any: Vec::new(),
            pricing: cade_ai::ModelPricing {
                input: input_per_m,
                output: output_per_m,
                cache_read: input_per_m * cr_ratio,
                cache_write: input_per_m * cw_ratio,
            },
        });
    }

    if rules.is_empty() {
        return Err("No priced models found in response".into());
    }

    Ok(rules)
}
