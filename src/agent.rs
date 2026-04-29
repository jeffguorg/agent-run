use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::{Map as JsonMap, Value as JsonValue};
use serde_yaml::{Mapping as YamlMapping, Value as YamlValue};
use toml::Value as TomlValue;

use crate::cli::Agent;
use crate::config::{
    ProviderConfig, cache_dir, claude_onboarding_lock_path, source_claude_state_path,
    source_codex_config_path, source_crush_config_path, source_hermes_config_path,
    try_create_lock_file,
};
use crate::error::AppError;
use crate::protocol::Protocol;

pub struct ResolvedLaunch<'a> {
    pub provider_name: &'a str,
    pub provider: &'a ProviderConfig,
    pub protocol: Protocol,
    pub key: String,
    pub model: String,
    pub agent_args: Vec<String>,
}

pub fn preferred_protocols(agent: Agent) -> &'static [Protocol] {
    match agent {
        Agent::Claude => &[Protocol::Anthropic],
        Agent::Codex => &[Protocol::OpenaiResponses],
        Agent::Hermes => &[Protocol::OpenaiChatCompletions],
        Agent::Crush => &[Protocol::OpenaiChatCompletions, Protocol::Anthropic],
    }
}

pub fn launch(agent: Agent, launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    match agent {
        Agent::Claude => launch_claude(launch),
        Agent::Codex => launch_codex(launch),
        Agent::Hermes => launch_hermes(launch),
        Agent::Crush => launch_crush(launch),
    }
}

fn launch_claude(launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    ensure_claude_onboarding_completed()?;
    let base_url = launch
        .provider
        .base_urls
        .anthropic
        .as_deref()
        .expect("anthropic base URL checked before launch");
    let mut cmd = Command::new("claude");
    cmd.arg("--model").arg(&launch.model);
    cmd.args(&launch.agent_args);
    cmd.env_remove("ANTHROPIC_API_KEY");
    cmd.env_remove("ANTHROPIC_AUTH_TOKEN");
    cmd.env_remove("ANTHROPIC_BASE_URL");
    if launch.provider.anthropic_use_api_key {
        cmd.env("ANTHROPIC_API_KEY", &launch.key);
    } else {
        cmd.env("ANTHROPIC_AUTH_TOKEN", &launch.key);
    }
    cmd.env("ANTHROPIC_BASE_URL", base_url);
    spawn_and_wait(cmd, "claude")
}

fn launch_codex(launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    let runtime_dir = codex_runtime_dir(launch.provider_name)?;
    fs::create_dir_all(&runtime_dir).map_err(AppError::TempDir)?;
    let config_path = runtime_dir.join("config.toml");
    let profile_name = "agent-run";
    let provider_name = sanitize_name(launch.provider_name);
    let merged_config =
        build_codex_runtime_config(&config_path, &provider_name, profile_name, &launch)?;
    let provider_toml =
        toml::to_string_pretty(&merged_config).map_err(|source| AppError::SerializeTomlConfig {
            path: config_path.clone(),
            source,
        })?;
    fs::write(&config_path, provider_toml).map_err(|source| AppError::WriteTempConfig {
        path: config_path.clone(),
        source,
    })?;

    let mut cmd = Command::new("codex");
    cmd.arg("--profile").arg(profile_name);
    cmd.args(&launch.agent_args);
    cmd.env("AGENT_RUN_OPENAI_API_KEY", &launch.key);
    cmd.env("CODEX_HOME", &runtime_dir);

    spawn_and_wait(cmd, "codex")
}

fn launch_hermes(launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    let runtime_dir = hermes_runtime_dir(launch.provider_name)?;
    fs::create_dir_all(&runtime_dir).map_err(AppError::TempDir)?;
    let config_path = runtime_dir.join("config.yaml");
    let merged_config = build_hermes_runtime_config(&config_path, &launch)?;
    let raw = serde_yaml::to_string(&merged_config).map_err(|source| {
        AppError::Message(format!(
            "failed to serialize Hermes config for {}: {source}",
            config_path.display()
        ))
    })?;
    fs::write(&config_path, raw).map_err(|source| AppError::WriteTempConfig {
        path: config_path.clone(),
        source,
    })?;

    let mut cmd = Command::new("hermes");
    cmd.args(&launch.agent_args);
    cmd.env("HERMES_HOME", &runtime_dir);
    cmd.env("OPENAI_API_KEY", &launch.key);

    spawn_and_wait(cmd, "hermes")
}

fn launch_crush(launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    let runtime_dir = crush_runtime_dir(launch.provider_name)?;
    let data_dir = runtime_dir.join("data");
    let config_dir = runtime_dir.join("config");
    fs::create_dir_all(&config_dir).map_err(AppError::TempDir)?;
    fs::create_dir_all(&data_dir).map_err(AppError::TempDir)?;

    let config_path = config_dir.join("crush.json");
    let merged_config = build_crush_runtime_config(&config_path, launch.provider_name, &launch)?;
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

    spawn_and_wait(cmd, "crush")
}

fn build_codex_runtime_config(
    runtime_config_path: &Path,
    provider_name: &str,
    profile_name: &str,
    launch: &ResolvedLaunch<'_>,
) -> Result<TomlValue, AppError> {
    let source_config_path = source_codex_config_path()?;
    let mut root = load_toml_config_if_exists(&source_config_path)?;
    if !root.is_table() {
        root = TomlValue::Table(toml::map::Map::new());
    }

    let generated = render_codex_config(provider_name, profile_name, &launch.model, launch);
    merge_toml_values(&mut root, generated);

    if runtime_config_path == source_config_path {
        return Err(AppError::Message(format!(
            "refusing to overwrite existing Codex config in place: {}",
            runtime_config_path.display()
        )));
    }

    Ok(root)
}

fn render_codex_config(
    provider_name: &str,
    profile_name: &str,
    model: &str,
    launch: &ResolvedLaunch<'_>,
) -> TomlValue {
    let base_url = launch
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

fn load_toml_config_if_exists(path: &Path) -> Result<TomlValue, AppError> {
    if !path.exists() {
        return Ok(TomlValue::Table(toml::map::Map::new()));
    }

    let raw = fs::read_to_string(path).map_err(|source| AppError::ReadConfig {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&raw).map_err(|source| AppError::ParseTomlConfig {
        path: path.to_path_buf(),
        source,
    })
}

fn build_crush_runtime_config(
    runtime_config_path: &Path,
    provider_name: &str,
    launch: &ResolvedLaunch<'_>,
) -> Result<JsonValue, AppError> {
    let source_config_path = source_crush_config_path()?;
    let mut root = load_json_config_if_exists(&source_config_path)?;
    if !root.is_object() {
        root = JsonValue::Object(JsonMap::new());
    }

    let generated = render_crush_config(provider_name, launch);
    merge_json_values(&mut root, generated);

    if runtime_config_path == source_config_path {
        return Err(AppError::Message(format!(
            "refusing to overwrite existing Crush config in place: {}",
            runtime_config_path.display()
        )));
    }

    Ok(root)
}

fn render_crush_config(provider_name: &str, launch: &ResolvedLaunch<'_>) -> JsonValue {
    let base_url = base_url_for_launch(launch);
    let provider_type = match launch.protocol {
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
    provider.insert("api_key".to_string(), JsonValue::String(launch.key.clone()));
    provider.insert(
        "models".to_string(),
        JsonValue::Array(vec![JsonValue::Object(JsonMap::from_iter([
            ("id".to_string(), JsonValue::String(launch.model.clone())),
            ("name".to_string(), JsonValue::String(launch.model.clone())),
        ]))]),
    );

    let default_key = match launch.protocol {
        Protocol::Anthropic => "default_large_model_id",
        Protocol::OpenaiChatCompletions | Protocol::OpenaiResponses => "default_large_model_id",
    };
    provider.insert(
        default_key.to_string(),
        JsonValue::String(launch.model.clone()),
    );
    provider.insert(
        "default_small_model_id".to_string(),
        JsonValue::String(launch.model.clone()),
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

fn merge_toml_values(base: &mut TomlValue, overlay: TomlValue) {
    match (base, overlay) {
        (TomlValue::Table(base_table), TomlValue::Table(overlay_table)) => {
            for (key, overlay_value) in overlay_table {
                if let Some(base_value) = base_table.get_mut(&key) {
                    merge_toml_values(base_value, overlay_value);
                } else {
                    base_table.insert(key, overlay_value);
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

fn build_hermes_runtime_config(
    runtime_config_path: &Path,
    launch: &ResolvedLaunch<'_>,
) -> Result<YamlValue, AppError> {
    let source_config_path = source_hermes_config_path()?;
    let mut root = load_yaml_config_if_exists(&source_config_path)?;
    if !matches!(root, YamlValue::Mapping(_)) {
        root = YamlValue::Mapping(YamlMapping::new());
    }

    let generated = render_hermes_config(&launch.model, launch);
    merge_yaml_values(&mut root, generated);

    if runtime_config_path == source_config_path {
        return Err(AppError::Message(format!(
            "refusing to overwrite existing Hermes config in place: {}",
            runtime_config_path.display()
        )));
    }

    Ok(root)
}

fn render_hermes_config(model: &str, launch: &ResolvedLaunch<'_>) -> YamlValue {
    let base_url = base_url_for_launch(launch);

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

fn base_url_for_launch<'a>(launch: &'a ResolvedLaunch<'a>) -> &'a str {
    match launch.protocol {
        Protocol::OpenaiChatCompletions | Protocol::OpenaiResponses => launch
            .provider
            .base_urls
            .openai
            .as_deref()
            .expect("openai base URL checked before launch"),
        Protocol::Anthropic => launch
            .provider
            .base_urls
            .anthropic
            .as_deref()
            .expect("anthropic base URL checked before launch"),
    }
}

fn load_yaml_config_if_exists(path: &Path) -> Result<YamlValue, AppError> {
    if !path.exists() {
        return Ok(YamlValue::Mapping(YamlMapping::new()));
    }
    let raw = fs::read_to_string(path).map_err(|source| AppError::ReadConfig {
        path: path.to_path_buf(),
        source,
    })?;
    serde_yaml::from_str(&raw).map_err(|source| AppError::ParseConfig {
        path: path.to_path_buf(),
        source,
    })
}

fn merge_yaml_values(base: &mut YamlValue, overlay: YamlValue) {
    match (base, overlay) {
        (YamlValue::Mapping(base_map), YamlValue::Mapping(overlay_map)) => {
            for (key, overlay_value) in overlay_map {
                if let Some(base_value) = base_map.get_mut(&key) {
                    merge_yaml_values(base_value, overlay_value);
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

fn spawn_and_wait(mut cmd: Command, program: &str) -> Result<ExitCode, AppError> {
    let status = cmd.status().map_err(|source| AppError::Spawn {
        program: program.to_string(),
        source,
    })?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' => ch,
            _ => '_',
        })
        .collect()
}
