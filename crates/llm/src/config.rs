//! Which models exist (`agents/models.yml`) and which of them this environment configured.
//!
//! The file names each model's environment variables instead of holding endpoints or keys, so
//! the same file works everywhere and secrets stay in `.env` or the secret store
//! (infra/README.md §5).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

use crate::chat::{ChatModel, ModelInfo};
use crate::openai::{OpenAiCompatible, StructuredOutput};
use crate::usage::Price;

/// `agents/models.yml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelsFile {
    /// Models by alias (`glimmer`, `spark`).
    pub models: BTreeMap<String, ModelConfig>,
}

/// One model: what to call it and where its endpoint comes from.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    /// Shown in the turn footer, for example `Muse Glimmer 30B`.
    pub display_name: String,
    /// Environment variable with the OpenAI-compatible base URL (`…/v1`).
    pub base_url_env: String,
    /// Environment variable with the provider's model id.
    pub model_env: String,
    /// Environment variable with the API key, when the endpoint needs one.
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub structured_output: StructuredOutput,
    /// Leave out when unknown; the call's cost is then not counted.
    #[serde(default)]
    pub price: Option<Price>,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

fn default_timeout() -> u64 {
    30
}

/// The models that are ready, and a sentence for each one that is not.
#[derive(Debug, Default, Clone)]
pub struct ModelSet {
    pub models: BTreeMap<String, Arc<dyn ChatModel>>,
    /// For example `spark: set META_API_KEY, SPARK_MODEL`.
    pub missing: Vec<String>,
}

impl ModelSet {
    pub fn get(&self, alias: &str) -> Option<&Arc<dyn ChatModel>> {
        self.models.get(alias)
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    /// The sentence a turn shows when no model can answer.
    pub fn not_configured_message(&self) -> String {
        let what = if self.missing.is_empty() {
            "add a model to agents/models.yml".to_owned()
        } else {
            self.missing.join("; ")
        };
        format!("No model is configured: {what} in .env, then restart the agent service.")
    }
}

impl ModelsFile {
    pub fn parse(yaml: &str) -> Result<Self, String> {
        serde_norway_parse(yaml)
    }

    /// Builds a client for every model whose variables are set. `env` returns `None` for an
    /// unset or empty variable.
    pub fn configure(&self, env: &dyn Fn(&str) -> Option<String>) -> ModelSet {
        let mut set = ModelSet::default();
        for (alias, c) in &self.models {
            let mut wanted = vec![c.base_url_env.as_str(), c.model_env.as_str()];
            wanted.extend(c.api_key_env.as_deref());
            let unset: Vec<&str> = wanted
                .iter()
                .copied()
                .filter(|v| env(v).is_none())
                .collect();
            if !unset.is_empty() {
                set.missing
                    .push(format!("{alias}: set {}", unset.join(", ")));
                continue;
            }
            let (Some(base_url), Some(model)) = (env(&c.base_url_env), env(&c.model_env)) else {
                continue;
            };
            let info = ModelInfo {
                alias: alias.clone(),
                model,
                display_name: c.display_name.clone(),
                price: c.price.clone(),
            };
            let api_key = c.api_key_env.as_deref().and_then(env);
            match OpenAiCompatible::new(
                info,
                &base_url,
                api_key,
                c.structured_output,
                Duration::from_secs(c.timeout_seconds),
            ) {
                Ok(client) => {
                    set.models.insert(alias.clone(), Arc::new(client));
                }
                Err(e) => set.missing.push(format!("{alias}: {e}")),
            }
        }
        set
    }
}

fn serde_norway_parse(yaml: &str) -> Result<ModelsFile, String> {
    serde_norway::from_str(yaml).map_err(|e| format!("agents/models.yml: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"
models:
  glimmer:
    display_name: Muse Glimmer 30B
    base_url_env: GLIMMER_BASE_URL
    model_env: GLIMMER_MODEL
    structured_output: guided_json
  spark:
    display_name: Muse Spark
    base_url_env: META_API_BASE_URL
    model_env: SPARK_MODEL
    api_key_env: META_API_KEY
    price: {input_per_million_tokens: "0.10", output_per_million_tokens: "0.30", currency: USD}
"#;

    #[test]
    fn configures_only_models_with_every_variable_set() {
        let file = ModelsFile::parse(FILE).unwrap();
        let env = |name: &str| match name {
            "GLIMMER_BASE_URL" => Some("http://models:8000/v1".to_owned()),
            "GLIMMER_MODEL" => Some("muse-glimmer-30b".to_owned()),
            "META_API_BASE_URL" => Some("https://api.example/v1".to_owned()),
            _ => None,
        };
        let set = file.configure(&env);
        assert_eq!(set.models.keys().collect::<Vec<_>>(), ["glimmer"]);
        assert_eq!(set.missing, ["spark: set SPARK_MODEL, META_API_KEY"]);
        let glimmer = set.get("glimmer").unwrap().info();
        assert_eq!(glimmer.model, "muse-glimmer-30b");
        assert_eq!(glimmer.display_name, "Muse Glimmer 30B");
    }

    #[test]
    fn says_what_to_set_when_nothing_is_configured() {
        let set = ModelsFile::parse(FILE).unwrap().configure(&|_| None);
        assert!(set.is_empty());
        let m = set.not_configured_message();
        assert!(m.starts_with("No model is configured: "), "{m}");
        assert!(
            m.contains("META_API_KEY") && m.contains("GLIMMER_BASE_URL"),
            "{m}"
        );
        assert!(ModelsFile::parse("models: {x: {display_name: X}}").is_err());
    }
}
