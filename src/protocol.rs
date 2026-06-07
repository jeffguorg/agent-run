use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, trace};

use crate::cli::ForceScopeSet;
use crate::config::ProviderConfig;
use crate::error::AppError;
use crate::model::{ModelSource, ModelSpec};

#[derive(Clone, Debug)]
pub struct FetchedModels {
    pub models: Vec<ModelSpec>,
    pub raw_response: Value,
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol {
    OpenaiChatCompletions,
    OpenaiResponses,
    Anthropic,
}

pub fn protocol_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenaiChatCompletions => "openai-chat-completions",
        Protocol::OpenaiResponses => "openai-responses",
        Protocol::Anthropic => "anthropic",
    }
}

pub fn model_list_url(protocol: Protocol, base_url: &str) -> String {
    match protocol {
        Protocol::OpenaiChatCompletions | Protocol::OpenaiResponses => {
            format!("{}/models", base_url.trim_end_matches('/'))
        }
        Protocol::Anthropic => format!("{}/v1/models", base_url.trim_end_matches('/')),
    }
}

pub fn base_url_for(provider: &ProviderConfig, protocol: Protocol) -> Option<&str> {
    match protocol {
        Protocol::OpenaiChatCompletions | Protocol::OpenaiResponses => {
            provider.base_urls.openai.as_deref()
        }
        Protocol::Anthropic => provider.base_urls.anthropic.as_deref(),
    }
}

pub fn resolve_key(provider: &ProviderConfig) -> Result<String, AppError> {
    if let Some(cmd) = provider.key_command.as_ref() {
        if cmd.is_empty() {
            return Err(AppError::EmptyKeyCommand);
        }
        let program = cmd[0].clone();
        let output = std::process::Command::new(&program)
            .args(&cmd[1..])
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .output()
            .map_err(|source| AppError::RunKeyCommand {
                command: cmd.join(" "),
                source,
            })?;
        if !output.status.success() {
            return Err(AppError::KeyCommandFailed {
                command: cmd.join(" "),
                status: output.status.code().unwrap_or(-1),
            });
        }
        let key = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if key.is_empty() {
            return Err(AppError::EmptyKeyOutput {
                command: cmd.join(" "),
            });
        }
        return Ok(key);
    }

    provider.key.clone().ok_or_else(|| {
        AppError::Message("provider must configure either `key_command` or `key`".to_string())
    })
}

pub fn fetch_models(
    protocol: Protocol,
    base_url: &str,
    key: &str,
) -> Result<FetchedModels, AppError> {
    match protocol {
        Protocol::OpenaiChatCompletions | Protocol::OpenaiResponses => {
            fetch_openai_models(base_url, key)
        }
        Protocol::Anthropic => fetch_anthropic_models(base_url, key),
    }
}

pub fn resolve_model(
    provider: &ProviderConfig,
    merged_models: &[String],
    requested: Option<&str>,
    force: ForceScopeSet,
) -> Result<String, AppError> {
    if merged_models.is_empty() {
        return requested.map(ToOwned::to_owned).ok_or_else(|| {
            AppError::Message("final model list is empty; pass `--model` explicitly".to_string())
        });
    }

    if let Some(default_model) = provider.default_model.as_deref()
        && !merged_models.iter().any(|model| model == default_model)
    {
        return Err(AppError::Message(format!(
            "default_model `{default_model}` is not present in the merged model list"
        )));
    }

    let resolved = requested
        .map(ToOwned::to_owned)
        .or_else(|| provider.default_model.clone())
        .ok_or_else(|| {
            AppError::Message(
                "missing model: configure `default_model` or pass `--model`".to_string(),
            )
        })?;

    if !force.model && !merged_models.iter().any(|model| model == &resolved) {
        return Err(AppError::Message(format!(
            "model `{resolved}` is not present in the merged model list; pass `--force model` to override"
        )));
    }

    Ok(resolved)
}

fn fetch_openai_models(base_url: &str, key: &str) -> Result<FetchedModels, AppError> {
    let url = model_list_url(Protocol::OpenaiResponses, base_url);
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| AppError::Message("invalid key for Authorization header".to_string()))?,
    );

    let value: Value = Client::new()
        .get(&url)
        .headers(headers)
        .send()
        .and_then(|resp| resp.error_for_status())
        .map_err(|source| AppError::Http {
            url: url.clone(),
            source,
        })?
        .json()
        .map_err(|source| AppError::Http {
            url: url.clone(),
            source,
        })?;

    normalize_remote_models(&url, value)
}

fn fetch_anthropic_models(base_url: &str, key: &str) -> Result<FetchedModels, AppError> {
    let url = model_list_url(Protocol::Anthropic, base_url);
    let value: Value = Client::new()
        .get(&url)
        .header("x-api-key", key)
        .header(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| {
                AppError::Message("invalid key for Authorization header".to_string())
            })?,
        )
        .header("anthropic-version", "2023-06-01")
        .send()
        .and_then(|resp| resp.error_for_status())
        .map_err(|source| AppError::Http {
            url: url.clone(),
            source,
        })?
        .json()
        .map_err(|source| AppError::Http {
            url: url.clone(),
            source,
        })?;

    normalize_remote_models(&url, value)
}

#[derive(Clone, Debug)]
struct RawModelCandidate<'a> {
    value: &'a Value,
    path: String,
}

#[derive(Clone, Debug)]
struct PartialModelSpec {
    id: Option<FieldValue<String>>,
    name: Option<FieldValue<String>>,
    context_window: Option<FieldValue<u64>>,
    reasoning: Option<FieldValue<bool>>,
    vision: Option<FieldValue<bool>>,
}

#[derive(Clone, Debug)]
struct FieldValue<T> {
    value: T,
    confidence: ExtractConfidence,
    extractor: &'static str,
    path: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum ExtractConfidence {
    WeakHint,
    StrongHint,
    Canonical,
}

fn normalize_remote_models(url: &str, value: Value) -> Result<FetchedModels, AppError> {
    let candidates = collect_model_candidates(&value);
    trace!(
        url,
        candidate_count = candidates.len(),
        "collected raw model candidates"
    );

    let mut models = Vec::new();
    for candidate in candidates {
        let partial = extract_model_spec(&candidate);
        let Some(id) = partial.id.as_ref().map(|field| field.value.clone()) else {
            trace!(url, path = %candidate.path, "dropping model candidate without id");
            continue;
        };
        let name_path = partial.name.as_ref().map(|field| field.path.clone());
        let context_window_path = partial
            .context_window
            .as_ref()
            .map(|field| field.path.clone());
        let reasoning_path = partial.reasoning.as_ref().map(|field| field.path.clone());
        let vision_path = partial.vision.as_ref().map(|field| field.path.clone());

        let model = ModelSpec {
            id,
            name: partial.name.map(|field| field.value),
            context_window: partial.context_window.map(|field| field.value),
            max_output_tokens: None,
            reasoning: partial.reasoning.map(|field| field.value),
            vision: partial.vision.map(|field| field.value),
            supports_attachments: None,
            input_cost_per_million: None,
            output_cost_per_million: None,
            cached_input_cost_per_million: None,
            cached_output_cost_per_million: None,
            source: ModelSource::Remote,
        };
        trace!(
            url,
            path = %candidate.path,
            model_id = %model.id,
            id_path = partial.id.as_ref().map(|field| field.path.clone()),
            name_path,
            context_window_path,
            reasoning_path,
            vision_path,
            effective_reasoning = model.effective_reasoning(),
            effective_vision = model.effective_vision(),
            has_name = model.name.is_some(),
            has_context_window = model.context_window.is_some(),
            "normalized remote model candidate"
        );
        models.push(model);
    }

    if models.is_empty() {
        return Err(AppError::ModelsResponse {
            url: url.to_string(),
            reason: "found no model objects with an `id`".to_string(),
        });
    }

    debug!(
        url,
        model_count = models.len(),
        "normalized remote model list"
    );
    Ok(FetchedModels {
        models,
        raw_response: value,
    })
}

fn collect_model_candidates<'a>(value: &'a Value) -> Vec<RawModelCandidate<'a>> {
    let mut candidates = Vec::new();
    collect_candidates_from_array(value.get("data"), "$.data", &mut candidates);
    collect_candidates_from_array(value.get("models"), "$.models", &mut candidates);
    collect_candidates_from_array(value.get("items"), "$.items", &mut candidates);
    collect_candidates_from_array(
        value.get("result").and_then(|result| result.get("models")),
        "$.result.models",
        &mut candidates,
    );

    if let Some(array) = value.as_array() {
        for (index, item) in array.iter().enumerate() {
            if item.is_object() {
                candidates.push(RawModelCandidate {
                    value: item,
                    path: format!("$[{index}]"),
                });
            }
        }
    }

    candidates
}

fn collect_candidates_from_array<'a>(
    value: Option<&'a Value>,
    base_path: &str,
    candidates: &mut Vec<RawModelCandidate<'a>>,
) {
    let Some(array) = value.and_then(Value::as_array) else {
        return;
    };

    for (index, item) in array.iter().enumerate() {
        if item.is_object() {
            candidates.push(RawModelCandidate {
                value: item,
                path: format!("{base_path}[{index}]"),
            });
        }
    }
}

fn extract_model_spec(candidate: &RawModelCandidate<'_>) -> PartialModelSpec {
    PartialModelSpec {
        id: select_best_string(
            candidate,
            &[
                ("id", ExtractConfidence::Canonical, "extract_id_from_id"),
                ("name", ExtractConfidence::WeakHint, "extract_id_from_name"),
            ],
        ),
        name: select_best_string(
            candidate,
            &[
                (
                    "name",
                    ExtractConfidence::Canonical,
                    "extract_name_from_name",
                ),
                (
                    "display_name",
                    ExtractConfidence::StrongHint,
                    "extract_name_from_display_name",
                ),
                (
                    "label",
                    ExtractConfidence::WeakHint,
                    "extract_name_from_label",
                ),
            ],
        ),
        context_window: extract_context_window(candidate),
        reasoning: extract_reasoning(candidate),
        vision: extract_vision(candidate),
    }
}

fn extract_context_window(candidate: &RawModelCandidate<'_>) -> Option<FieldValue<u64>> {
    let mut best = select_best_u64(
        candidate,
        &[
            (
                "context_window",
                ExtractConfidence::Canonical,
                "extract_context_window_from_context_window",
            ),
            (
                "context_length",
                ExtractConfidence::StrongHint,
                "extract_context_window_from_context_length",
            ),
            (
                "input_token_limit",
                ExtractConfidence::StrongHint,
                "extract_context_window_from_input_token_limit",
            ),
        ],
    );

    if let Some(value) = nested_u64(candidate.value, &["top_provider", "context_length"]) {
        update_best_field(
            &mut best,
            FieldValue {
                value,
                confidence: ExtractConfidence::StrongHint,
                extractor: "extract_context_window_from_top_provider_context_length",
                path: format!("{}.top_provider.context_length", candidate.path),
            },
        );
    }

    if let Some((path, value)) = token_limits_max(candidate.value) {
        update_best_field(
            &mut best,
            FieldValue {
                value,
                confidence: ExtractConfidence::StrongHint,
                extractor: "extract_context_window_from_token_limits",
                path: format!("{}.{}", candidate.path, path),
            },
        );
    }

    best
}

fn extract_reasoning(candidate: &RawModelCandidate<'_>) -> Option<FieldValue<bool>> {
    let mut best = select_best_bool(
        candidate,
        &[
            (
                "reasoning",
                ExtractConfidence::Canonical,
                "extract_reasoning_from_reasoning",
            ),
            (
                "supports_reasoning",
                ExtractConfidence::StrongHint,
                "extract_reasoning_from_supports_reasoning",
            ),
        ],
        Some("reasoning"),
    );

    if let Some(value) = string_array_contains(
        candidate.value,
        &["supported_parameters"],
        &["reasoning", "include_reasoning"],
    ) {
        update_best_field(
            &mut best,
            FieldValue {
                value,
                confidence: ExtractConfidence::StrongHint,
                extractor: "extract_reasoning_from_supported_parameters",
                path: format!("{}.supported_parameters", candidate.path),
            },
        );
    }

    best
}

fn extract_vision(candidate: &RawModelCandidate<'_>) -> Option<FieldValue<bool>> {
    let mut best = select_best_bool(
        candidate,
        &[
            (
                "vision",
                ExtractConfidence::Canonical,
                "extract_vision_from_vision",
            ),
            (
                "supports_vision",
                ExtractConfidence::StrongHint,
                "extract_vision_from_supports_vision",
            ),
            (
                "supports_image_in",
                ExtractConfidence::StrongHint,
                "extract_vision_from_supports_image_in",
            ),
        ],
        Some("vision"),
    );

    if let Some(value) = string_array_contains(
        candidate.value,
        &["architecture", "input_modalities"],
        &["image"],
    ) {
        update_best_field(
            &mut best,
            FieldValue {
                value,
                confidence: ExtractConfidence::StrongHint,
                extractor: "extract_vision_from_architecture_input_modalities",
                path: format!("{}.architecture.input_modalities", candidate.path),
            },
        );
    }

    if let Some(value) = string_array_contains(candidate.value, &["modalities"], &["image"]) {
        update_best_field(
            &mut best,
            FieldValue {
                value,
                confidence: ExtractConfidence::StrongHint,
                extractor: "extract_vision_from_modalities",
                path: format!("{}.modalities", candidate.path),
            },
        );
    }

    if let Some(value) = nested_str(candidate.value, &["architecture", "modality"])
        && value.contains("image")
    {
        update_best_field(
            &mut best,
            FieldValue {
                value: true,
                confidence: ExtractConfidence::WeakHint,
                extractor: "extract_vision_from_architecture_modality",
                path: format!("{}.architecture.modality", candidate.path),
            },
        );
    }

    best
}

fn select_best_string(
    candidate: &RawModelCandidate<'_>,
    strategies: &[(&str, ExtractConfidence, &'static str)],
) -> Option<FieldValue<String>> {
    let mut best = None;

    for (field, confidence, extractor) in strategies {
        let Some(value) = candidate.value.get(*field).and_then(Value::as_str) else {
            continue;
        };
        let proposed = FieldValue {
            value: value.to_string(),
            confidence: *confidence,
            extractor,
            path: format!("{}.{}", candidate.path, field),
        };
        trace!(
            candidate_path = %candidate.path,
            extractor,
            field,
            confidence = ?confidence,
            value = %proposed.value,
            "string extractor matched"
        );
        update_best_field(&mut best, proposed);
    }

    best
}

fn select_best_u64(
    candidate: &RawModelCandidate<'_>,
    strategies: &[(&str, ExtractConfidence, &'static str)],
) -> Option<FieldValue<u64>> {
    let mut best = None;

    for (field, confidence, extractor) in strategies {
        let Some(value) = candidate.value.get(*field).and_then(Value::as_u64) else {
            continue;
        };
        let proposed = FieldValue {
            value,
            confidence: *confidence,
            extractor,
            path: format!("{}.{}", candidate.path, field),
        };
        trace!(
            candidate_path = %candidate.path,
            extractor,
            field,
            confidence = ?confidence,
            value,
            "u64 extractor matched"
        );
        update_best_field(&mut best, proposed);
    }

    best
}

fn select_best_bool(
    candidate: &RawModelCandidate<'_>,
    strategies: &[(&str, ExtractConfidence, &'static str)],
    capability_key: Option<&str>,
) -> Option<FieldValue<bool>> {
    let mut best = None;

    for (field, confidence, extractor) in strategies {
        if let Some(value) = candidate.value.get(*field).and_then(Value::as_bool) {
            let proposed = FieldValue {
                value,
                confidence: *confidence,
                extractor,
                path: format!("{}.{}", candidate.path, field),
            };
            trace!(
                candidate_path = %candidate.path,
                extractor,
                field,
                confidence = ?confidence,
                value,
                "bool extractor matched"
            );
            update_best_field(&mut best, proposed);
        }
    }

    if best.is_none()
        && let Some(capability_key) = capability_key
        && let Some(value) = bool_from_capabilities(candidate.value, capability_key)
    {
        let proposed = FieldValue {
            value,
            confidence: ExtractConfidence::WeakHint,
            extractor: "extract_bool_from_capabilities",
            path: format!("{}.capabilities.{capability_key}", candidate.path),
        };
        trace!(
            candidate_path = %candidate.path,
            capability_key,
            value,
            "capabilities bool extractor matched"
        );
        update_best_field(&mut best, proposed);
    }

    best
}

fn bool_from_capabilities(value: &Value, key: &str) -> Option<bool> {
    value
        .get("capabilities")
        .and_then(Value::as_object)
        .and_then(|capabilities| capabilities.get(key))
        .and_then(Value::as_bool)
}

fn nested_u64(value: &Value, path: &[&str]) -> Option<u64> {
    nested_value(value, path).and_then(Value::as_u64)
}

fn nested_str<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    nested_value(value, path).and_then(Value::as_str)
}

fn nested_value<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn string_array_contains(value: &Value, path: &[&str], needles: &[&str]) -> Option<bool> {
    let items = nested_value(value, path)?.as_array()?;
    Some(items.iter().filter_map(Value::as_str).any(|item| {
        needles
            .iter()
            .any(|needle| item.eq_ignore_ascii_case(needle))
    }))
}

fn token_limits_max(value: &Value) -> Option<(String, u64)> {
    let token_limits = value.get("token_limits")?.as_object()?;
    let mut best = None;
    for (key, entry) in token_limits {
        match entry {
            Value::Number(number) => {
                if let Some(value) = number.as_u64() {
                    update_token_limit_best(&mut best, format!("token_limits.{key}"), value);
                }
            }
            Value::Object(object) => {
                for (nested_key, nested_entry) in object {
                    if let Some(value) = nested_entry.as_u64() {
                        update_token_limit_best(
                            &mut best,
                            format!("token_limits.{key}.{nested_key}"),
                            value,
                        );
                    }
                }
            }
            _ => {}
        }
    }
    best
}

fn update_token_limit_best(best: &mut Option<(String, u64)>, path: String, value: u64) {
    match best {
        Some((_, existing)) if *existing >= value => {}
        _ => *best = Some((path, value)),
    }
}

fn update_best_field<T>(best: &mut Option<FieldValue<T>>, proposed: FieldValue<T>) {
    match best {
        Some(existing) if existing.confidence > proposed.confidence => {}
        Some(existing)
            if existing.confidence == proposed.confidence
                && existing.extractor <= proposed.extractor => {}
        _ => {
            *best = Some(proposed);
        }
    }
}
