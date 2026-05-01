use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use tracing::{debug, trace, warn};

use crate::config::cache_dir;
use crate::error::AppError;
use crate::protocol::{Protocol, protocol_name};

const MODEL_CACHE_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const REASONING_DEFAULT: bool = true;
const VISION_DEFAULT: bool = false;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum RawModelConfig {
    String(String),
    Object(RawModelObject),
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct RawModelObject {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub vision: Option<bool>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ModelSource {
    LocalString,
    LocalObject,
    Remote,
    Cache,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelSpec {
    pub id: String,
    pub name: Option<String>,
    pub context_window: Option<u64>,
    pub reasoning: Option<bool>,
    pub vision: Option<bool>,
    pub source: ModelSource,
}

impl ModelSpec {
    pub fn effective_reasoning(&self) -> bool {
        self.reasoning.unwrap_or(REASONING_DEFAULT)
    }

    pub fn effective_vision(&self) -> bool {
        self.vision.unwrap_or(VISION_DEFAULT)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedModelStore {
    pub version: u32,
    pub provider: String,
    pub protocol: String,
    pub models: Vec<CachedModelSpec>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedModelSpec {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub vision: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct ModelCacheSnapshot {
    pub path: PathBuf,
    pub protocol: Option<Protocol>,
    pub is_fresh: bool,
    pub models: Vec<ModelSpec>,
}

#[derive(Clone, Debug, Default)]
pub struct LoadRemoteModels {
    pub models: Vec<ModelSpec>,
    pub used_cache: bool,
}

pub fn normalize_local_models(raw_models: &[RawModelConfig]) -> Vec<ModelSpec> {
    raw_models
        .iter()
        .map(|raw_model| match raw_model {
            RawModelConfig::String(id) => {
                let model = ModelSpec {
                    id: id.clone(),
                    name: None,
                    context_window: None,
                    reasoning: None,
                    vision: None,
                    source: ModelSource::LocalString,
                };
                trace!(model_id = %model.id, "normalized local string model");
                model
            }
            RawModelConfig::Object(raw) => {
                let model = ModelSpec {
                    id: raw.id.clone(),
                    name: raw.name.clone(),
                    context_window: raw.context_window,
                    reasoning: raw.reasoning,
                    vision: raw.vision,
                    source: ModelSource::LocalObject,
                };
                trace!(
                    model_id = %model.id,
                    has_name = model.name.is_some(),
                    has_context_window = model.context_window.is_some(),
                    has_reasoning = model.reasoning.is_some(),
                    has_vision = model.vision.is_some(),
                    "normalized local object model"
                );
                model
            }
        })
        .collect()
}

pub fn merge_models(local: &[ModelSpec], discovered: &[ModelSpec]) -> Vec<ModelSpec> {
    let mut merged: BTreeMap<String, ModelSpec> = BTreeMap::new();

    for model in local.iter().chain(discovered.iter()) {
        if let Some(existing) = merged.get_mut(&model.id) {
            let replacement = merge_duplicate_models(existing.clone(), model.clone());
            trace!(
                model_id = %model.id,
                existing_source = ?existing.source,
                incoming_source = ?model.source,
                selected_source = ?replacement.source,
                "merged duplicate model entry"
            );
            *existing = replacement;
        } else {
            merged.insert(model.id.clone(), model.clone());
        }
    }

    let merged = merged.into_values().collect::<Vec<_>>();
    debug!(model_count = merged.len(), "built merged model list");
    merged
}

pub fn model_ids(models: &[ModelSpec]) -> Vec<String> {
    models.iter().map(|model| model.id.clone()).collect()
}

pub fn model_cache_path(provider_name: &str) -> Result<PathBuf, AppError> {
    Ok(cache_dir()?
        .join("agent-run")
        .join("model")
        .join(format!("{}.json", sanitize_name(provider_name))))
}

pub fn load_model_cache(provider_name: &str) -> Result<Option<ModelCacheSnapshot>, AppError> {
    let path = model_cache_path(provider_name)?;
    if !path.exists() {
        trace!(provider_name, path = %path.display(), "model cache file does not exist");
        return Ok(None);
    }

    let raw = fs::read_to_string(&path).map_err(|source| AppError::ReadCache {
        path: path.clone(),
        source,
    })?;
    let store: CachedModelStore =
        serde_json::from_str(&raw).map_err(|source| AppError::ParseCache {
            path: path.clone(),
            source,
        })?;
    let metadata = fs::metadata(&path).map_err(|source| AppError::ReadCache {
        path: path.clone(),
        source,
    })?;
    let modified = metadata.modified().map_err(|source| AppError::ReadCache {
        path: path.clone(),
        source,
    })?;
    let now = SystemTime::now();
    let age = now.duration_since(modified).unwrap_or_default();
    let is_fresh = age <= MODEL_CACHE_TTL;
    trace!(
        provider_name,
        path = %path.display(),
        age_seconds = age.as_secs(),
        is_fresh,
        protocol = %store.protocol,
        model_count = store.models.len(),
        "loaded model cache metadata"
    );

    Ok(Some(ModelCacheSnapshot {
        path,
        protocol: parse_cached_protocol(&store.protocol),
        is_fresh,
        models: store
            .models
            .into_iter()
            .map(|model| ModelSpec {
                id: model.id,
                name: model.name,
                context_window: model.context_window,
                reasoning: model.reasoning,
                vision: model.vision,
                source: ModelSource::Cache,
            })
            .collect(),
    }))
}

pub fn write_model_cache(
    provider_name: &str,
    protocol: Protocol,
    remote_models: &[ModelSpec],
) -> Result<(), AppError> {
    let path = model_cache_path(provider_name)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(AppError::TempDir)?;
    }

    let store = CachedModelStore {
        version: 1,
        provider: provider_name.to_string(),
        protocol: protocol_name(protocol).to_string(),
        models: remote_models
            .iter()
            .map(|model| CachedModelSpec {
                id: model.id.clone(),
                name: model.name.clone(),
                context_window: model.context_window,
                reasoning: model.reasoning,
                vision: model.vision,
            })
            .collect(),
    };

    let raw = serde_json::to_string_pretty(&store).map_err(|source| AppError::SerializeCache {
        path: path.clone(),
        source,
    })?;
    fs::write(&path, raw).map_err(|source| AppError::WriteCache {
        path: path.clone(),
        source,
    })?;
    debug!(
        provider_name,
        protocol = protocol_name(protocol),
        path = %path.display(),
        model_count = remote_models.len(),
        "wrote model cache"
    );
    Ok(())
}

pub fn load_cached_remote_models(
    provider_name: &str,
    protocol: Protocol,
    allow_stale: bool,
) -> Result<Option<Vec<ModelSpec>>, AppError> {
    let Some(snapshot) = load_model_cache(provider_name)? else {
        return Ok(None);
    };

    if snapshot.protocol == Some(protocol) && snapshot.is_fresh {
        debug!(
            provider_name,
            protocol = protocol_name(protocol),
            path = %snapshot.path.display(),
            model_count = snapshot.models.len(),
            "using fresh model cache"
        );
        return Ok(Some(snapshot.models));
    }

    if allow_stale {
        warn!(
            provider_name,
            requested_protocol = protocol_name(protocol),
            cached_protocol = snapshot.protocol.map(protocol_name),
            path = %snapshot.path.display(),
            is_fresh = snapshot.is_fresh,
            "using stale or protocol-mismatched model cache"
        );
        return Ok(Some(snapshot.models));
    }

    trace!(
        provider_name,
        requested_protocol = protocol_name(protocol),
        cached_protocol = snapshot.protocol.map(protocol_name),
        path = %snapshot.path.display(),
        is_fresh = snapshot.is_fresh,
        "cache not eligible for direct use"
    );
    Ok(None)
}

pub fn describe_model_help(
    default_model: Option<&str>,
    local_models: &[ModelSpec],
    discovered_models: &[ModelSpec],
    model_id: &str,
) -> String {
    let is_default = default_model == Some(model_id);
    let is_local = local_models.iter().any(|model| model.id == model_id);
    let discovered_label = discovered_models
        .iter()
        .find(|model| model.id == model_id)
        .map(|model| match model.source {
            ModelSource::Cache => "cached",
            ModelSource::Remote => "remote",
            ModelSource::LocalString | ModelSource::LocalObject => "model",
        });

    match (is_default, is_local, discovered_label) {
        (true, true, Some(label)) => format!("default + configured + {label}"),
        (true, true, None) => "default + configured".to_string(),
        (true, false, Some(label)) => format!("default + {label}"),
        (true, false, None) => "default".to_string(),
        (false, true, Some(label)) => format!("configured + {label}"),
        (false, true, None) => "configured".to_string(),
        (false, false, Some(label)) => label.to_string(),
        (false, false, None) => "model".to_string(),
    }
}

fn merge_duplicate_models(existing: ModelSpec, incoming: ModelSpec) -> ModelSpec {
    use ModelSource::{Cache, LocalObject, LocalString, Remote};

    match (existing.source, incoming.source) {
        (LocalObject, _) => merge_preferred(existing, incoming),
        (_, LocalObject) => merge_preferred(incoming, existing),
        (Remote, _) => merge_preferred(existing, incoming),
        (_, Remote) => merge_preferred(incoming, existing),
        (Cache, LocalString) => merge_preferred(existing, incoming),
        (LocalString, Cache) => merge_preferred(incoming, existing),
        _ => merge_preferred(incoming, existing),
    }
}

fn merge_preferred(preferred: ModelSpec, fallback: ModelSpec) -> ModelSpec {
    ModelSpec {
        id: preferred.id,
        name: preferred.name.or(fallback.name),
        context_window: preferred.context_window.or(fallback.context_window),
        reasoning: preferred.reasoning.or(fallback.reasoning),
        vision: preferred.vision.or(fallback.vision),
        source: preferred.source,
    }
}

fn parse_cached_protocol(name: &str) -> Option<Protocol> {
    match name {
        "openai-chat-completions" => Some(Protocol::OpenaiChatCompletions),
        "openai-responses" => Some(Protocol::OpenaiResponses),
        "anthropic" => Some(Protocol::Anthropic),
        _ => None,
    }
}

fn sanitize_name(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    sanitized.trim_matches('_').to_string()
}

pub fn log_cache_fallback(
    provider_name: &str,
    protocol: Protocol,
    cache_path: &Path,
    url: &str,
    err: &AppError,
) {
    warn!(
        provider_name,
        protocol = protocol_name(protocol),
        cache_path = %cache_path.display(),
        url,
        error = %err,
        "model API fetch failed; falling back to cached models"
    );
}
