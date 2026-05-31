use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::{Map as JsonMap, Value as JsonValue};
use serde_yaml::{Mapping as YamlMapping, Value as YamlValue};
use toml::Value as TomlValue;

use crate::cli::{Agent, agent_name};
use crate::config::{
    ProviderConfig, cache_dir, claude_onboarding_lock_path, skeleton_dir, source_claude_state_path,
    source_crush_config_path, try_create_lock_file,
};
use crate::error::AppError;
use crate::protocol::{Protocol, protocol_name};

pub struct ManagedProviderLaunch<'a> {
    pub provider: &'a ProviderConfig,
    pub protocol: Protocol,
    pub key: String,
    pub model: String,
}

pub struct ResolvedLaunch<'a> {
    pub agent: Agent,
    pub provider_name: &'a str,
    pub managed_provider: Option<ManagedProviderLaunch<'a>>,
    pub agent_args: Vec<String>,
}

pub fn preferred_protocols(agent: Agent) -> &'static [Protocol] {
    match agent {
        Agent::Claude => &[Protocol::Anthropic],
        Agent::Codex => &[Protocol::OpenaiResponses],
        Agent::Hermes => &[Protocol::OpenaiChatCompletions],
        Agent::Crush => &[Protocol::OpenaiChatCompletions, Protocol::Anthropic],
        Agent::Shell => &[Protocol::Anthropic, Protocol::OpenaiChatCompletions, Protocol::OpenaiResponses],
    }
}

pub fn launch(agent: Agent, launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    match agent {
        Agent::Claude => launch_claude(launch),
        Agent::Codex => launch_codex(launch),
        Agent::Hermes => launch_hermes(launch),
        Agent::Crush => launch_crush(launch),
        Agent::Shell => launch_shell(launch),
    }
}

fn launch_claude(launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    ensure_claude_onboarding_completed()?;
    let managed = launch.managed_provider.as_ref().ok_or_else(|| {
        AppError::Message("claude launch requires a resolved provider".to_string())
    })?;
    let base_url = managed
        .provider
        .base_urls
        .anthropic
        .as_deref()
        .expect("anthropic base URL checked before launch");
    let mut cmd = Command::new("claude");
    cmd.arg("--model").arg(&managed.model);
    cmd.args(&launch.agent_args);
    cmd.env_remove("ANTHROPIC_API_KEY");
    cmd.env_remove("ANTHROPIC_AUTH_TOKEN");
    cmd.env_remove("ANTHROPIC_BASE_URL");
    if managed.provider.anthropic_use_api_key {
        cmd.env("ANTHROPIC_API_KEY", &managed.key);
    } else {
        cmd.env("ANTHROPIC_AUTH_TOKEN", &managed.key);
    }
    cmd.env("ANTHROPIC_BASE_URL", base_url);
    apply_extra_env(&mut cmd, &launch)?;
    spawn_and_wait(cmd, "claude")
}

fn launch_codex(launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    let runtime_dir = codex_runtime_dir(launch.provider_name)?;
    prepare_runtime_home(&runtime_dir, "codex")?;

    let mut cmd = Command::new("codex");
    if let Some(managed) = launch.managed_provider.as_ref() {
        let config_path = runtime_dir.join("config.toml");
        let profile_name = "agent-run";
        let provider_name = sanitize_name(launch.provider_name);
        let generated = render_codex_config(&provider_name, profile_name, &managed.model, managed);
        let provider_toml =
            toml::to_string_pretty(&generated).map_err(|source| AppError::SerializeTomlConfig {
                path: config_path.clone(),
                source,
            })?;
        fs::write(&config_path, provider_toml).map_err(|source| AppError::WriteTempConfig {
            path: config_path.clone(),
            source,
        })?;
        cmd.arg("--profile").arg(profile_name);
        cmd.env("AGENT_RUN_OPENAI_API_KEY", &managed.key);
    }
    cmd.args(&launch.agent_args);
    cmd.env("CODEX_HOME", &runtime_dir);
    apply_extra_env(&mut cmd, &launch)?;

    spawn_and_wait(cmd, "codex")
}

fn launch_hermes(launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    let runtime_dir = hermes_runtime_dir(launch.provider_name)?;
    prepare_runtime_home(&runtime_dir, "hermes")?;
    if let Some(managed) = launch.managed_provider.as_ref() {
        let config_path = runtime_dir.join("config.yaml");
        let generated = render_hermes_config(&managed.model, managed);
        let raw = serde_yaml::to_string(&generated).map_err(|source| {
            AppError::Message(format!(
                "failed to serialize Hermes config for {}: {source}",
                config_path.display()
            ))
        })?;
        fs::write(&config_path, raw).map_err(|source| AppError::WriteTempConfig {
            path: config_path.clone(),
            source,
        })?;
    }

    let mut cmd = Command::new("hermes");
    cmd.args(&launch.agent_args);
    cmd.env("HERMES_HOME", &runtime_dir);
    if let Some(managed) = launch.managed_provider.as_ref() {
        cmd.env("OPENAI_API_KEY", &managed.key);
    }
    apply_extra_env(&mut cmd, &launch)?;

    spawn_and_wait(cmd, "hermes")
}

fn launch_crush(launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    let managed = launch.managed_provider.as_ref().ok_or_else(|| {
        AppError::Message("crush launch requires a resolved provider".to_string())
    })?;
    let runtime_dir = crush_runtime_dir(launch.provider_name)?;
    let data_dir = runtime_dir.join("data");
    let config_dir = runtime_dir.join("config");
    fs::create_dir_all(&config_dir).map_err(AppError::TempDir)?;
    fs::create_dir_all(&data_dir).map_err(AppError::TempDir)?;

    let config_path = config_dir.join("crush.json");
    let merged_config = build_crush_runtime_config(&config_path, launch.provider_name, managed)?;
    let raw = serde_json::to_string_pretty(&merged_config).map_err(|source| {
        AppError::Message(format!("failed to serialize Crush config: {source}"))
    })?;
    fs::write(&config_path, raw).map_err(|source| AppError::WriteTempConfig {
        path: config_path.clone(),
        source,
    })?;

    let mut cmd = Command::new("crush");
    cmd.arg("--data-dir").arg(&data_dir);
    cmd.args(&launch.agent_args);
    cmd.env("CRUSH_GLOBAL_CONFIG", &config_dir);
    cmd.env("CRUSH_GLOBAL_DATA", &data_dir);
    apply_extra_env(&mut cmd, &launch)?;

    spawn_and_wait(cmd, "crush")
}

fn render_codex_config(
    provider_name: &str,
    profile_name: &str,
    model: &str,
    managed: &ManagedProviderLaunch<'_>,
) -> TomlValue {
    let base_url = managed
        .provider
        .base_urls
        .openai
        .as_deref()
        .expect("openai base URL checked before launch");
    toml::Value::Table(toml::map::Map::from_iter([
        (
            "model_providers".to_string(),
            toml::Value::Table(toml::map::Map::from_iter([(
                provider_name.to_string(),
                toml::Value::Table(toml::map::Map::from_iter([
                    (
                        "name".to_string(),
                        toml::Value::String(provider_name.to_string()),
                    ),
                    (
                        "base_url".to_string(),
                        toml::Value::String(base_url.to_string()),
                    ),
                    (
                        "env_key".to_string(),
                        toml::Value::String("AGENT_RUN_OPENAI_API_KEY".to_string()),
                    ),
                    (
                        "wire_api".to_string(),
                        toml::Value::String("responses".to_string()),
                    ),
                ])),
            )])),
        ),
        (
            "profiles".to_string(),
            toml::Value::Table(toml::map::Map::from_iter([(
                profile_name.to_string(),
                toml::Value::Table(toml::map::Map::from_iter([
                    ("model".to_string(), toml::Value::String(model.to_string())),
                    (
                        "model_provider".to_string(),
                        toml::Value::String(provider_name.to_string()),
                    ),
                ])),
            )])),
        ),
    ]))
}

fn codex_runtime_dir(provider_name: &str) -> Result<PathBuf, AppError> {
    Ok(cache_dir()?
        .join("agent-run")
        .join("codex")
        .join(sanitize_name(provider_name)))
}

fn hermes_runtime_dir(provider_name: &str) -> Result<PathBuf, AppError> {
    Ok(cache_dir()?
        .join("agent-run")
        .join("hermes")
        .join(sanitize_name(provider_name)))
}

fn crush_runtime_dir(provider_name: &str) -> Result<PathBuf, AppError> {
    Ok(cache_dir()?
        .join("agent-run")
        .join("crush")
        .join(sanitize_name(provider_name)))
}

fn build_crush_runtime_config(
    runtime_config_path: &Path,
    provider_name: &str,
    managed: &ManagedProviderLaunch<'_>,
) -> Result<JsonValue, AppError> {
    let source_config_path = source_crush_config_path()?;
    let mut root = load_json_config_if_exists(&source_config_path)?;
    if !root.is_object() {
        root = JsonValue::Object(JsonMap::new());
    }

    let generated = render_crush_config(provider_name, managed);
    merge_json_values(&mut root, generated);

    if runtime_config_path == source_config_path {
        return Err(AppError::Message(format!(
            "refusing to overwrite existing Crush config in place: {}",
            runtime_config_path.display()
        )));
    }

    Ok(root)
}

fn render_crush_config(provider_name: &str, managed: &ManagedProviderLaunch<'_>) -> JsonValue {
    let base_url = base_url_for_managed_launch(managed);
    let provider_type = match managed.protocol {
        Protocol::OpenaiChatCompletions | Protocol::OpenaiResponses => "openai-compat",
        Protocol::Anthropic => "anthropic",
    };

    let mut provider = JsonMap::new();
    provider.insert(
        "type".to_string(),
        JsonValue::String(provider_type.to_string()),
    );
    provider.insert(
        "base_url".to_string(),
        JsonValue::String(base_url.to_string()),
    );
    provider.insert(
        "api_key".to_string(),
        JsonValue::String(managed.key.clone()),
    );
    provider.insert(
        "models".to_string(),
        JsonValue::Array(vec![JsonValue::Object(JsonMap::from_iter([
            ("id".to_string(), JsonValue::String(managed.model.clone())),
            ("name".to_string(), JsonValue::String(managed.model.clone())),
        ]))]),
    );

    let default_key = match managed.protocol {
        Protocol::Anthropic => "default_large_model_id",
        Protocol::OpenaiChatCompletions | Protocol::OpenaiResponses => "default_large_model_id",
    };
    provider.insert(
        default_key.to_string(),
        JsonValue::String(managed.model.clone()),
    );
    provider.insert(
        "default_small_model_id".to_string(),
        JsonValue::String(managed.model.clone()),
    );

    let mut providers = JsonMap::new();
    providers.insert(provider_name.to_string(), JsonValue::Object(provider));

    JsonValue::Object(JsonMap::from_iter([(
        "providers".to_string(),
        JsonValue::Object(providers),
    )]))
}

fn load_json_config_if_exists(path: &Path) -> Result<JsonValue, AppError> {
    if !path.exists() {
        return Ok(JsonValue::Object(JsonMap::new()));
    }

    let raw = fs::read_to_string(path).map_err(|source| AppError::ReadConfig {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&raw).map_err(|source| {
        AppError::Message(format!(
            "failed to parse JSON config from {}: {source}",
            path.display()
        ))
    })
}

fn merge_json_values(base: &mut JsonValue, overlay: JsonValue) {
    match (base, overlay) {
        (JsonValue::Object(base_map), JsonValue::Object(overlay_map)) => {
            for (key, overlay_value) in overlay_map {
                if let Some(base_value) = base_map.get_mut(&key) {
                    merge_json_values(base_value, overlay_value);
                } else {
                    base_map.insert(key, overlay_value);
                }
            }
        }
        (base_slot, overlay_value) => {
            *base_slot = overlay_value;
        }
    }
}

fn ensure_claude_onboarding_completed() -> Result<(), AppError> {
    let lock_path = claude_onboarding_lock_path()?;
    let should_modify = try_create_lock_file(&lock_path)?;
    if !should_modify {
        return Ok(());
    }

    let state_path = source_claude_state_path()?;
    let mut json = match fs::read_to_string(&state_path) {
        Ok(raw) => match serde_json::from_str::<JsonValue>(&raw) {
            Ok(JsonValue::Object(map)) => JsonValue::Object(map),
            Ok(_) | Err(_) => {
                backup_if_exists(&state_path)?;
                JsonValue::Object(JsonMap::new())
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => JsonValue::Object(JsonMap::new()),
        Err(err) => {
            return Err(AppError::ReadConfig {
                path: state_path,
                source: err,
            });
        }
    };

    let object = json.as_object_mut().expect("json object constructed above");
    let current = object
        .get("hasCompletedOnboarding")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    if !current {
        object.insert("hasCompletedOnboarding".to_string(), JsonValue::Bool(true));
        let raw = serde_json::to_string_pretty(&json).map_err(|source| {
            AppError::Message(format!("failed to serialize Claude state JSON: {source}"))
        })?;
        fs::write(&state_path, raw).map_err(|source| AppError::WriteTempConfig {
            path: state_path,
            source,
        })?;
    }

    Ok(())
}

fn backup_if_exists(path: &Path) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    let backup_path = path.with_extension("json.agent-run.bak");
    fs::copy(path, &backup_path).map_err(|source| AppError::WriteTempConfig {
        path: backup_path,
        source,
    })?;
    Ok(())
}

fn render_hermes_config(model: &str, managed: &ManagedProviderLaunch<'_>) -> YamlValue {
    let base_url = base_url_for_managed_launch(managed);

    let mut model_map = YamlMapping::new();
    model_map.insert(
        YamlValue::String("default".to_string()),
        YamlValue::String(model.to_string()),
    );
    model_map.insert(
        YamlValue::String("provider".to_string()),
        YamlValue::String("custom".to_string()),
    );
    model_map.insert(
        YamlValue::String("base_url".to_string()),
        YamlValue::String(base_url.to_string()),
    );

    let mut root = YamlMapping::new();
    root.insert(
        YamlValue::String("model".to_string()),
        YamlValue::Mapping(model_map),
    );
    YamlValue::Mapping(root)
}

fn base_url_for_managed_launch<'a>(managed: &'a ManagedProviderLaunch<'a>) -> &'a str {
    match managed.protocol {
        Protocol::OpenaiChatCompletions | Protocol::OpenaiResponses => managed
            .provider
            .base_urls
            .openai
            .as_deref()
            .expect("openai base URL checked before launch"),
        Protocol::Anthropic => managed
            .provider
            .base_urls
            .anthropic
            .as_deref()
            .expect("anthropic base URL checked before launch"),
    }
}

fn spawn_and_wait(mut cmd: Command, program: &str) -> Result<ExitCode, AppError> {
    let status = cmd.status().map_err(|source| AppError::Spawn {
        program: program.to_string(),
        source,
    })?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn apply_extra_env(cmd: &mut Command, launch: &ResolvedLaunch<'_>) -> Result<(), AppError> {
    let Some(managed) = launch.managed_provider.as_ref() else {
        return Ok(());
    };

    for (key, template) in &managed.provider.extra_env {
        let value = expand_extra_env_value(key, template, launch)?;
        cmd.env(key, value);
    }
    Ok(())
}

fn expand_extra_env_value(
    key: &str,
    template: &str,
    launch: &ResolvedLaunch<'_>,
) -> Result<String, AppError> {
    let mut result = String::with_capacity(template.len());
    let mut remaining = template;

    while let Some(pos) = remaining.find("${") {
        result.push_str(&remaining[..pos]);
        let after_open = &remaining[pos + 2..];
        let close = after_open.find('}').ok_or_else(|| {
            AppError::Message(format!(
                "extra_env value for `{key}` is `{template}` and has an unterminated `${{...}}`"
            ))
        })?;
        let placeholder = &after_open[..close];
        let (namespace, name) = placeholder.split_once(':').ok_or_else(|| {
            AppError::Message(format!(
                "extra_env value for `{key}` contains placeholder `${{{placeholder}}}` without `ns:name` form"
            ))
        })?;

        let resolved = match namespace {
            "env" => env::var(name).map_err(|_| {
                AppError::Message(format!(
                    "extra_env value for `{key}` references unset env var `{name}`"
                ))
            })?,
            "context" => resolve_context_field(name, launch).map_err(|err| match err {
                AppError::Message(msg) => AppError::Message(format!("extra_env `{key}`: {msg}")),
                other => other,
            })?,
            other => {
                return Err(AppError::Message(format!(
                    "extra_env value for `{key}` uses unknown namespace `{other}`; expected `env` or `context`"
                )));
            }
        };

        result.push_str(&resolved);
        remaining = &after_open[close + 1..];
    }

    result.push_str(remaining);
    Ok(result)
}

fn resolve_context_field(name: &str, launch: &ResolvedLaunch<'_>) -> Result<String, AppError> {
    match name {
        "provider" => Ok(launch.provider_name.to_string()),
        "protocol" => Ok(protocol_name(
            launch
                .managed_provider
                .as_ref()
                .ok_or_else(|| {
                    AppError::Message(
                        "context field `protocol` is unavailable without a resolved provider"
                            .to_string(),
                    )
                })?
                .protocol,
        )
        .to_string()),
        "model" => Ok(launch
            .managed_provider
            .as_ref()
            .ok_or_else(|| {
                AppError::Message(
                    "context field `model` is unavailable without a resolved provider".to_string(),
                )
            })?
            .model
            .clone()),
        "key" => Ok(launch
            .managed_provider
            .as_ref()
            .ok_or_else(|| {
                AppError::Message(
                    "context field `key` is unavailable without a resolved provider".to_string(),
                )
            })?
            .key
            .clone()),
        "agent" => Ok(agent_name(launch.agent).to_string()),
        "base_url" => Ok(base_url_for_managed_launch(
            launch.managed_provider.as_ref().ok_or_else(|| {
                AppError::Message(
                    "context field `base_url` is unavailable without a resolved provider"
                        .to_string(),
                )
            })?,
        )
        .to_string()),
        other => Err(AppError::Message(format!(
            "unknown context field `{other}`; supported: provider, protocol, model, key, agent, base_url"
        ))),
    }
}

fn prepare_runtime_home(runtime_dir: &Path, agent_name: &str) -> Result<(), AppError> {
    if runtime_dir.exists() {
        fs::remove_dir_all(runtime_dir).map_err(AppError::TempDir)?;
    }
    fs::create_dir_all(runtime_dir).map_err(AppError::TempDir)?;
    copy_skeleton_into(runtime_dir, agent_name)
}

fn copy_skeleton_into(runtime_dir: &Path, agent_name: &str) -> Result<(), AppError> {
    let skeleton_dir = skeleton_dir(agent_name)?;
    if !skeleton_dir.exists() {
        return Ok(());
    }
    copy_dir_contents(&skeleton_dir, runtime_dir)
}

fn copy_dir_contents(source: &Path, destination: &Path) -> Result<(), AppError> {
    for entry in fs::read_dir(source).map_err(AppError::TempDir)? {
        let entry = entry.map_err(AppError::TempDir)?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type().map_err(AppError::TempDir)?;
        if file_type.is_dir() {
            fs::create_dir_all(&destination_path).map_err(AppError::TempDir)?;
            copy_dir_contents(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent).map_err(AppError::TempDir)?;
            }
            fs::copy(&source_path, &destination_path).map_err(|source| {
                AppError::WriteTempConfig {
                    path: destination_path.clone(),
                    source,
                }
            })?;
        }
    }
    Ok(())
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' => ch,
            _ => '_',
        })
        .collect()
}

/// Collect env var key-value pairs for the "set everything" pattern used by
/// `launch shell` and `export-env`.
pub fn collect_shell_env(launch: &ResolvedLaunch<'_>) -> Result<Vec<(String, String)>, AppError> {
    let managed = launch.managed_provider.as_ref().ok_or_else(|| {
        AppError::Message("shell env requires a resolved provider".to_string())
    })?;
    let provider = managed.provider;
    let mut envs = Vec::new();

    // Base URLs for both protocol families
    if let Some(url) = provider.base_urls.anthropic.as_deref() {
        envs.push(("ANTHROPIC_BASE_URL".to_string(), url.to_string()));
    }
    if let Some(url) = provider.base_urls.openai.as_deref() {
        envs.push(("OPENAI_BASE_URL".to_string(), url.to_string()));
    }

    // API keys based on provider config
    if provider.anthropic_use_api_key {
        envs.push(("ANTHROPIC_API_KEY".to_string(), managed.key.clone()));
    } else {
        envs.push(("ANTHROPIC_AUTH_TOKEN".to_string(), managed.key.clone()));
    }
    envs.push(("OPENAI_API_KEY".to_string(), managed.key.clone()));

    // Model env vars
    envs.push(("ANTHROPIC_MODEL".to_string(), managed.model.clone()));
    envs.push(("OPENAI_MODEL".to_string(), managed.model.clone()));

    // Extra env from provider config
    for (key, template) in &provider.extra_env {
        let value = expand_extra_env_value(key, template, launch)?;
        envs.push((key.clone(), value));
    }

    Ok(envs)
}

/// Escape a value for shell single-quoting: wrap in `'…'`, escaping embedded `'` as `'\''`.
pub fn shell_escape(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

fn launch_shell(launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    launch.managed_provider.as_ref().ok_or_else(|| {
        AppError::Message("shell launch requires a resolved provider".to_string())
    })?;

    // Determine shell path: use $SHELL unless first arg looks like a shell path (contains /)
    // or is a known shell name without leading dash
    let (shell_path, shell_exec_args) = if launch.agent_args.is_empty() {
        let shell = env::var("SHELL").map_err(|_| {
            AppError::Message("SHELL env var is not set; pass the shell explicitly".to_string())
        })?;
        (shell, vec![])
    } else if launch.agent_args[0].contains('/') || !launch.agent_args[0].starts_with('-') {
        (launch.agent_args[0].clone(), launch.agent_args[1..].to_vec())
    } else {
        let shell = env::var("SHELL").map_err(|_| {
            AppError::Message("SHELL env var is not set; pass the shell explicitly".to_string())
        })?;
        (shell, launch.agent_args.clone())
    };

    let mut cmd = Command::new(&shell_path);
    cmd.args(&shell_exec_args);

    // Clear stale values from parent environment before setting new ones
    cmd.env_remove("ANTHROPIC_API_KEY");
    cmd.env_remove("ANTHROPIC_AUTH_TOKEN");
    cmd.env_remove("ANTHROPIC_BASE_URL");
    cmd.env_remove("OPENAI_API_KEY");
    cmd.env_remove("OPENAI_BASE_URL");

    for (key, value) in collect_shell_env(&launch)? {
        cmd.env(key, value);
    }

    spawn_and_wait(cmd, &shell_path)
}
