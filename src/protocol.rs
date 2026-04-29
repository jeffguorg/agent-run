use std::collections::BTreeSet;

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;

use crate::cli::ForceScopeSet;
use crate::config::ProviderConfig;
use crate::error::AppError;

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
) -> Result<Vec<String>, AppError> {
    match protocol {
        Protocol::OpenaiChatCompletions | Protocol::OpenaiResponses => {
            fetch_openai_models(base_url, key)
        }
        Protocol::Anthropic => fetch_anthropic_models(base_url, key),
    }
}

pub fn merge_models(configured: &[String], discovered: &[String]) -> Vec<String> {
    let mut merged = BTreeSet::new();
    merged.extend(configured.iter().cloned());
    merged.extend(discovered.iter().cloned());
    merged.into_iter().collect()
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

fn fetch_openai_models(base_url: &str, key: &str) -> Result<Vec<String>, AppError> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
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

    let data =
        value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| AppError::ModelsResponse {
                url: url.clone(),
                reason: "missing `data` array".to_string(),
            })?;

    let mut models = Vec::new();
    for item in data {
        if let Some(id) = item.get("id").and_then(Value::as_str) {
            models.push(id.to_string());
        }
    }
    Ok(models)
}

fn fetch_anthropic_models(base_url: &str, key: &str) -> Result<Vec<String>, AppError> {
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let value: Value = Client::new()
        .get(&url)
        .header("x-api-key", key)
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

    let data =
        value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| AppError::ModelsResponse {
                url: url.clone(),
                reason: "missing `data` array".to_string(),
            })?;

    let mut models = Vec::new();
    for item in data {
        if let Some(id) = item.get("id").and_then(Value::as_str) {
            models.push(id.to_string());
        }
    }
    Ok(models)
}
