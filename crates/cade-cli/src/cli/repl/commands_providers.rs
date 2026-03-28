use super::Repl;
use crate::Result;
use std::io;

impl Repl {
    /// Interactive /connect flow — guided provider setup.
    pub(crate) async fn handle_connect(&self, preset: Option<String>, _stdout: &mut io::Stdout) -> Result<()> {
        use crate::ui::question::{Question, QuestionOption};

        const BUILTIN: &[(&str, &str)] = &[
            ("anthropic", "Anthropic (Claude models)"),
            ("openai", "OpenAI (GPT / Codex models)"),
            ("gemini", "Google Gemini"),
            ("ollama", "Ollama (local models, no key needed)"),
        ];

        let presets = self.client.list_provider_presets().await;

        let (name, kind, default_base_url) = if let Some(p) = preset {
            if let Some(&(n, _)) = BUILTIN.iter().find(|(n, _)| *n == p.as_str()) {
                (n.to_string(), n.to_string(), None)
            } else if let Some(preset_val) = presets.iter().find(|v| v["name"].as_str() == Some(&p))
            {
                let base = preset_val["base_url"].as_str().map(String::from);
                (p.clone(), "openai-compatible".to_string(), base)
            } else {
                (p.clone(), "openai-compatible".to_string(), None)
            }
        } else {
            // Interactive picker via QuestionWidget
            let mut all_options: Vec<(String, String, Option<String>)> = BUILTIN
                .iter()
                .map(|(n, label)| (n.to_string(), label.to_string(), None))
                .collect();
            for p in &presets {
                let n = p["name"].as_str().unwrap_or("?").to_string();
                let u = p["base_url"].as_str().map(String::from);
                all_options.push((n.clone(), format!("{n} (OpenAI-compatible)"), u));
            }
            all_options.push((
                "custom".to_string(),
                "Custom OpenAI-compatible URL…".to_string(),
                None,
            ));

            let opts: Vec<QuestionOption> = all_options
                .iter()
                .map(|(_, label, _)| QuestionOption {
                    label: label.clone(),
                    description: String::new(),
                })
                .collect();
            let q = Question {
                header: "Connect provider".to_string(),
                text: "Choose a provider to connect:".to_string(),
                options: opts.clone(),
                multi_select: false,
                allow_other: false,
                progress: None,
            };
            let ans = {
                let mut app = self.app.lock().expect("lock poisoned");
                app.ask_question(&q)?
            };
            let Some(chosen) = ans else {
                return Ok(());
            };
            let label = chosen.as_str();
            let idx = all_options
                .iter()
                .position(|(_, l, _)| l.as_str() == label)
                .unwrap_or(0);
            let (n, _, base) = all_options.remove(idx);
            let k = if BUILTIN.iter().any(|(bn, _)| *bn == n.as_str()) {
                n.clone()
            } else {
                "openai-compatible".to_string()
            };
            (n, k, base)
        };

        // Ask for API key
        let needs_key = kind != "ollama";
        let api_key = if needs_key {
            let key_opts = vec![QuestionOption {
                label: "Skip (no key)".to_string(),
                description: String::new(),
            }];
            let kq = Question {
                header: "API Key".to_string(),
                text: format!("API key for '{name}' (type key or select Skip):"),
                options: key_opts.clone(),
                multi_select: false,
                allow_other: true,
                progress: None,
            };
            let ans = {
                let mut app = self.app.lock().expect("lock poisoned");
                app.ask_question(&kq)?
            };
            match &ans {
                Some(a) if a.as_str() != "Skip (no key)" && !a.as_str().is_empty() => {
                    Some(a.as_str().to_string())
                }
                _ => None,
            }
        } else {
            None
        };

        // Ask for base URL if needed
        let base_url = if kind == "openai-compatible" && default_base_url.is_none() {
            let url_opts = vec![QuestionOption {
                label: "Cancel".to_string(),
                description: String::new(),
            }];
            let uq = Question {
                header: "Base URL".to_string(),
                text: "Base URL (e.g. https://api.example.com/v1):".to_string(),
                options: url_opts.clone(),
                multi_select: false,
                allow_other: true,
                progress: None,
            };
            let ans = {
                let mut app = self.app.lock().expect("lock poisoned");
                app.ask_question(&uq)?
            };
            match &ans {
                Some(a) if a.as_str() != "Cancel" && !a.as_str().is_empty() => {
                    Some(a.as_str().to_string())
                }
                _ => None,
            }
        } else {
            default_base_url
        };

        self.tui_dim(format!("  Connecting to '{name}'…"));
        match self
            .client
            .add_provider(&name, &kind, api_key.as_deref(), base_url.as_deref())
            .await
        {
            Ok(_) => {
                self.tui_ok(format!("  ✓ Provider '{name}' connected and hot-loaded"));
                self.tui_dim(format!("    Use: /model {name}/<model-name>"));
            }
            Err(e) => self.tui_err(e.to_string()),
        }
        Ok(())
    }
}
