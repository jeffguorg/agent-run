use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::{Map as JsonMap, Value as JsonValue};
use serde_yaml::{Mapping as YamlMapping, Value as YamlValue};
use toml::Value as TomlValue;
use tracing::warn;

use crate::cli::{Agent, agent_name};
use crate::config::{
    ProviderConfig, cache_dir, claude_onboarding_lock_path, source_claude_state_path,
    source_crush_config_path, try_create_lock_file,
};
use crate::error::AppError;
use crate::model::ModelSpec;
use crate::protocol::{Protocol, protocol_name};

pub struct ManagedProviderLaunch<'a> {
    pub provider: &'a ProviderConfig,
    pub protocol: Protocol,
    pub key: String,
    pub models: Vec<ModelSpec>,
}

pub struct ResolvedLaunch<'a> {
    pub agent: Agent,
    pub target_provider: &'a str,
    pub target_model: Option<String>,
    pub configured_providers: BTreeSet<String>,
    pub providers: BTreeMap<String, ManagedProviderLaunch<'a>>,
    pub agent_args: Vec<String>,
}

impl<'a> ResolvedLaunch<'a> {
    pub fn target(&self) -> Option<&ManagedProviderLaunch<'a>> {
        self.providers.get(self.target_provider)
    }
}

impl Agent {
    pub fn needs_all_providers(&self) -> bool {
        matches!(self, Agent::Crush)
    }
}

pub fn preferred_protocols(agent: Agent) -> &'static [Protocol] {
    match agent {
        Agent::Claude => &[Protocol::Anthropic],
        Agent::Codex => &[Protocol::OpenaiResponses],
        Agent::Hermes => &[Protocol::OpenaiChatCompletions],
        Agent::Crush => &[Protocol::OpenaiChatCompletions, Protocol::Anthropic],
        Agent::Shell => &[
            Protocol::Anthropic,
            Protocol::OpenaiChatCompletions,
            Protocol::OpenaiResponses,
        ],
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
    let Some(target) = launch.target() else {
        unreachable!("agent-run bug: claude launch requires a resolved provider");
    };
    let Some(model) = launch.target_model.as_ref() else {
        unreachable!("agent-run bug: claude launch requires a resolved model");
    };
    let base_url = target
        .provider
        .base_urls
        .anthropic
        .as_deref()
        .expect("anthropic base URL checked before launch");
    let mut cmd = Command::new("claude");
    cmd.arg("--model").arg(model);
    cmd.args(&launch.agent_args);
    cmd.env_remove("ANTHROPIC_API_KEY");
    cmd.env_remove("ANTHROPIC_AUTH_TOKEN");
    cmd.env_remove("ANTHROPIC_BASE_URL");
    if target.provider.anthropic_use_api_key {
        cmd.env("ANTHROPIC_API_KEY", &target.key);
    } else {
        cmd.env("ANTHROPIC_AUTH_TOKEN", &target.key);
    }
    cmd.env("ANTHROPIC_BASE_URL", base_url);
    apply_extra_env(&mut cmd, &launch)?;
    spawn_and_wait(cmd, "claude")
}

fn launch_codex(launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    let runtime_dir = codex_runtime_dir(launch.target_provider)?;
    fs::create_dir_all(&runtime_dir).map_err(AppError::TempDir)?;

    let mut cmd = Command::new("codex");
    let config_path = runtime_dir.join("config.toml");
    if let (Some(target), Some(model)) = (launch.target(), launch.target_model.as_ref()) {
        let profile_name = "agent-run";
        let provider_name = sanitize_name(launch.target_provider);
        let generated = render_codex_config(&provider_name, profile_name, model, target);
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
        cmd.env("AGENT_RUN_OPENAI_API_KEY", &target.key);
    } else {
        merge_codex_isolated_config(&config_path)?;
    }
    cmd.args(&launch.agent_args);
    cmd.env("CODEX_HOME", &runtime_dir);
    apply_extra_env(&mut cmd, &launch)?;

    spawn_and_wait(cmd, "codex")
}

fn merge_codex_isolated_config(config_path: &Path) -> Result<(), AppError> {
    let raw = if config_path.exists() {
        fs::read_to_string(config_path).map_err(|source| AppError::ReadConfig {
            path: config_path.to_path_buf(),
            source,
        })?
    } else {
        String::new()
    };

    let mut value: toml::Value = toml::from_str(&raw).map_err(|source| AppError::ParseTomlConfig {
        path: config_path.to_path_buf(),
        source,
    })?;
    if let Some(table) = value.as_table_mut() {
        table.insert(
            "cli_auth_credentials_store".to_string(),
            toml::Value::String("file".to_string()),
        );
    }
    let updated = toml::to_string_pretty(&value).map_err(|source| AppError::SerializeTomlConfig {
        path: config_path.to_path_buf(),
        source,
    })?;
    fs::write(config_path, updated).map_err(|source| AppError::WriteTempConfig {
        path: config_path.to_path_buf(),
        source,
    })?;

    Ok(())
}

fn launch_hermes(launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    let runtime_dir = hermes_runtime_dir(launch.target_provider)?;
    fs::create_dir_all(&runtime_dir).map_err(AppError::TempDir)?;
    if let (Some(target), Some(model)) = (launch.target(), launch.target_model.as_ref()) {
        let config_path = runtime_dir.join("config.yaml");
        let generated = render_hermes_config(model, target);
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
    if let Some(target) = launch.target() {
        cmd.env("OPENAI_API_KEY", &target.key);
    }
    apply_extra_env(&mut cmd, &launch)?;

    spawn_and_wait(cmd, "hermes")
}

fn launch_crush(launch: ResolvedLaunch<'_>) -> Result<ExitCode, AppError> {
    if launch.target().is_none() {
        unreachable!("agent-run bug: crush launch requires a resolved provider");
    }

    sync_crush_config(&launch.providers, &launch.configured_providers)?;

    if let Some(model) = launch.target_model.as_deref() {
        warn!(
            provider = launch.target_provider,
            requested_model = model,
            "crush does not support setting the launch model through agent-run; model selection is left to Crush defaults, command flags, or UI"
        );
    }

    let mut cmd = Command::new("crush");
    cmd.args(&launch.agent_args);
    for (name, managed) in &launch.providers {
        cmd.env(provider_api_key_env_name(name), &managed.key);
    }
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

fn provider_api_key_env_name(provider_name: &str) -> String {
    let upper: String = provider_name
        .to_uppercase()
        .chars()
        .map(|ch| {
            matches!(ch, 'A'..='Z' | '0'..='9')
                .then_some(ch)
                .unwrap_or('_')
        })
        .collect();
    format!("{upper}_API_KEY")
}

fn provider_api_key_template(provider_name: &str) -> String {
    let env_var = provider_api_key_env_name(provider_name);
    format!(
        "${{{env_var}:?this provider is managed by agent-run, use agent-run launch {provider_name} crush to launch crush}}"
    )
}

fn render_crush_provider_entry(
    provider_name: &str,
    managed: &ManagedProviderLaunch<'_>,
) -> JsonValue {
    let base_url = base_url_for_managed_launch(managed);
    let provider_type = match managed.protocol {
        Protocol::OpenaiChatCompletions | Protocol::OpenaiResponses => "openai-compat",
        Protocol::Anthropic => "anthropic",
    };

    let mut entry = JsonMap::new();
    entry.insert(
        "$agent-run-managed".to_string(),
        JsonValue::String(provider_name.to_string()),
    );
    entry.insert(
        "type".to_string(),
        JsonValue::String(provider_type.to_string()),
    );
    entry.insert(
        "base_url".to_string(),
        JsonValue::String(base_url.to_string()),
    );
    entry.insert(
        "api_key".to_string(),
        JsonValue::String(provider_api_key_template(provider_name)),
    );
    if !managed.models.is_empty() {
        let mut seen = BTreeSet::new();
        let mut models = Vec::new();
        for model in &managed.models {
            if !seen.insert(model.id.clone()) {
                continue;
            }

            let mut model_entry = JsonMap::new();
            model_entry.insert("id".to_string(), JsonValue::String(model.id.clone()));
            model_entry.insert(
                "name".to_string(),
                JsonValue::String(model.name.clone().unwrap_or_else(|| model.id.clone())),
            );
            models.push(JsonValue::Object(model_entry));
        }
        entry.insert("models".to_string(), JsonValue::Array(models));
    }

    JsonValue::Object(entry)
}

fn agent_run_managed_provider_name(value: &JsonValue) -> Option<&str> {
    value.get("$agent-run-managed").and_then(JsonValue::as_str)
}

fn sync_crush_config(
    providers: &BTreeMap<String, ManagedProviderLaunch<'_>>,
    configured_providers: &BTreeSet<String>,
) -> Result<(), AppError> {
    // Check env var name collisions
    let mut env_var_names: std::collections::HashMap<String, &str> =
        std::collections::HashMap::new();
    for name in providers.keys() {
        let env_name = provider_api_key_env_name(name);
        if let Some(existing) = env_var_names.insert(env_name, name) {
            return Err(AppError::Message(format!(
                "providers `{existing}` and `{name}` map to the same API key env var; rename one"
            )));
        }
    }

    let config_path = source_crush_config_path()?;
    let mut root = load_json_config_if_exists(&config_path)?;
    if !root.is_object() {
        root = JsonValue::Object(JsonMap::new());
    }

    let root_obj = root.as_object_mut().expect("checked above");
    root_obj.remove("provider");
    if !root_obj.contains_key("providers") {
        root_obj.insert("providers".to_string(), JsonValue::Object(JsonMap::new()));
    }
    if !root_obj.contains_key("options") {
        root_obj.insert("options".to_string(), JsonValue::Object(JsonMap::new()));
    }
    let options = root_obj
        .get_mut("options")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| {
            AppError::Message(format!(
                "Crush config `{}` has non-object `options`; cannot merge agent-run options",
                config_path.display()
            ))
        })?;
    options.insert(
        "disable_default_providers".to_string(),
        JsonValue::Bool(true),
    );
    options.insert(
        "disable_provider_auto_update".to_string(),
        JsonValue::Bool(true),
    );
    let crush_providers = root_obj
        .get_mut("providers")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| {
            AppError::Message(format!(
                "Crush config `{}` has non-object `providers`; cannot merge agent-run providers",
                config_path.display()
            ))
        })?;

    for name in providers.keys() {
        match crush_providers.get(name) {
            Some(existing) if agent_run_managed_provider_name(existing).is_none() => {
                return Err(AppError::Message(format!(
                    "crush provider `{name}` exists and is not managed by agent-run; \
                     remove it from crush config or rename the agent-run provider"
                )));
            }
            _ => {}
        }
    }

    // Remove stale managed providers (in crush.json but no longer in agent-run config)
    let stale: Vec<String> = crush_providers
        .iter()
        .filter(|(name, value)| {
            agent_run_managed_provider_name(value)
                .is_some_and(|managed_name| managed_name == name.as_str())
                && !configured_providers.contains(name.as_str())
        })
        .map(|(k, _)| k.clone())
        .collect();
    for name in stale {
        crush_providers.remove(&name);
    }

    // Write/update managed providers
    for (name, managed) in providers {
        crush_providers.insert(name.clone(), render_crush_provider_entry(name, managed));
    }

    // Write back to disk
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(AppError::TempDir)?;
    }
    let raw = serde_json::to_string_pretty(&root).map_err(|source| {
        AppError::Message(format!("failed to serialize Crush config: {source}"))
    })?;
    fs::write(&config_path, raw).map_err(|source| AppError::WriteTempConfig {
        path: config_path.clone(),
        source,
    })?;

    Ok(())
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
    let Some(target) = launch.target() else {
        return Ok(());
    };

    for (key, template) in &target.provider.extra_env {
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
        "provider" => Ok(launch.target_provider.to_string()),
        "protocol" => {
            let target = launch.target().ok_or_else(|| {
                AppError::Message(
                    "context field `protocol` is unavailable without a resolved provider"
                        .to_string(),
                )
            })?;
            Ok(protocol_name(target.protocol).to_string())
        }
        "model" => launch.target_model.as_ref().cloned().ok_or_else(|| {
            AppError::Message(
                "context field `model` is unavailable without a resolved model".to_string(),
            )
        }),
        "key" => {
            let target = launch.target().ok_or_else(|| {
                AppError::Message(
                    "context field `key` is unavailable without a resolved provider".to_string(),
                )
            })?;
            Ok(target.key.clone())
        }
        "agent" => Ok(agent_name(launch.agent).to_string()),
        "base_url" => {
            let target = launch.target().ok_or_else(|| {
                AppError::Message(
                    "context field `base_url` is unavailable without a resolved provider"
                        .to_string(),
                )
            })?;
            Ok(base_url_for_managed_launch(target).to_string())
        }
        other => Err(AppError::Message(format!(
            "unknown context field `{other}`; supported: provider, protocol, model, key, agent, base_url"
        ))),
    }
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' => ch,
            _ => '_',
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// Collect env var key-value pairs for the "set everything" pattern used by
/// `launch shell` and `export-env`.
pub fn collect_shell_env(launch: &ResolvedLaunch<'_>) -> Result<Vec<(String, String)>, AppError> {
    let target = launch
        .target()
        .ok_or_else(|| AppError::Message("shell env requires a resolved provider".to_string()))?;
    let model = launch
        .target_model
        .as_ref()
        .ok_or_else(|| AppError::Message("shell env requires a resolved model".to_string()))?;
    let provider = target.provider;
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
        envs.push(("ANTHROPIC_API_KEY".to_string(), target.key.clone()));
    } else {
        envs.push(("ANTHROPIC_AUTH_TOKEN".to_string(), target.key.clone()));
    }
    envs.push(("OPENAI_API_KEY".to_string(), target.key.clone()));

    // Model env vars
    envs.push(("ANTHROPIC_MODEL".to_string(), model.clone()));
    envs.push(("OPENAI_MODEL".to_string(), model.clone()));

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
    launch.target().ok_or_else(|| {
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
        (
            launch.agent_args[0].clone(),
            launch.agent_args[1..].to_vec(),
        )
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;
    use crate::config::{BaseUrls, ProviderConfig};
    use crate::model::{ModelApiFilterConfig, ModelSource, ModelSpec, RawModelConfig};

    fn test_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn provider(extra_env: &[(&str, &str)]) -> ProviderConfig {
        ProviderConfig {
            protocols: vec![Protocol::OpenaiChatCompletions],
            base_urls: BaseUrls {
                openai: Some("https://example.test/v1".to_string()),
                anthropic: None,
            },
            key: Some("test-key".to_string()),
            key_command: None,
            anthropic_use_api_key: false,
            default_model: Some("gpt-4.1".to_string()),
            models: vec![RawModelConfig::String("gpt-4.1".to_string())],
            model_api_filters: ModelApiFilterConfig::Disabled,
            legacy_disable_model_loading_from_api: None,
            extra_env: extra_env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn managed_provider<'a>(provider: &'a ProviderConfig) -> ManagedProviderLaunch<'a> {
        ManagedProviderLaunch {
            provider,
            protocol: Protocol::OpenaiChatCompletions,
            key: "test-key".to_string(),
            models: vec![],
        }
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("agent-run-{label}-{nanos}"))
    }

    #[test]
    fn sync_crush_config_keeps_managed_provider_missing_from_current_launch() {
        let _guard = test_env_lock().lock().expect("env lock poisoned");
        let config_root = unique_temp_dir("crush-sync");
        fs::create_dir_all(&config_root).expect("should create temp config root");
        let config_path = config_root.join("crush.json");
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&json!({
                "provider": "target",
                "providers": {
                    "target": {
                        "$agent-run-managed": "target",
                        "type": "openai-compat",
                        "base_url": "https://example.test/v1",
                        "api_key": "${TARGET_API_KEY:?managed}"
                    },
                    "other": {
                        "$agent-run-managed": "other",
                        "type": "openai-compat",
                        "base_url": "https://example.test/v1",
                        "api_key": "${OTHER_API_KEY:?managed}"
                    }
                }
            }))
            .expect("json should serialize"),
        )
        .expect("should write initial crush config");

        // SAFETY: tests hold a process-wide mutex while mutating environment variables.
        unsafe {
            std::env::set_var("CRUSH_GLOBAL_CONFIG", &config_root);
        }

        let target_provider = provider(&[]);
        let mut providers = BTreeMap::new();
        providers.insert("target".to_string(), managed_provider(&target_provider));
        let configured_providers = ["target".to_string(), "other".to_string()]
            .into_iter()
            .collect();

        sync_crush_config(&providers, &configured_providers).expect("sync should succeed");

        let updated: JsonValue = serde_json::from_str(
            &fs::read_to_string(&config_path).expect("should read synced crush config"),
        )
        .expect("synced config should parse");
        assert!(updated.get("provider").is_none());
        assert_eq!(
            updated["options"]["disable_default_providers"].as_bool(),
            Some(true)
        );
        assert_eq!(
            updated["options"]["disable_provider_auto_update"].as_bool(),
            Some(true)
        );
        let names = updated["providers"]
            .as_object()
            .expect("providers should be an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert!(names.iter().any(|name| name == "other"));

        // SAFETY: tests hold a process-wide mutex while mutating environment variables.
        unsafe {
            std::env::remove_var("CRUSH_GLOBAL_CONFIG");
        }
        let _ = fs::remove_dir_all(&config_root);
    }

    #[test]
    fn render_crush_provider_entry_writes_minimal_model_shape() {
        let provider = provider(&[]);
        let managed = ManagedProviderLaunch {
            provider: &provider,
            protocol: Protocol::OpenaiChatCompletions,
            key: "test-key".to_string(),
            models: vec![
                ModelSpec {
                    id: "gpt-4.1".to_string(),
                    name: Some("GPT-4.1".to_string()),
                    context_window: Some(123),
                    max_output_tokens: Some(456),
                    reasoning: Some(true),
                    vision: Some(true),
                    supports_attachments: Some(true),
                    input_cost_per_million: Some(1.0),
                    output_cost_per_million: Some(2.0),
                    cached_input_cost_per_million: Some(3.0),
                    cached_output_cost_per_million: Some(4.0),
                    source: ModelSource::LocalObject,
                },
                ModelSpec {
                    id: "gpt-4.1-mini".to_string(),
                    name: None,
                    context_window: None,
                    max_output_tokens: None,
                    reasoning: None,
                    vision: None,
                    supports_attachments: None,
                    input_cost_per_million: None,
                    output_cost_per_million: None,
                    cached_input_cost_per_million: None,
                    cached_output_cost_per_million: None,
                    source: ModelSource::LocalString,
                },
                ModelSpec {
                    id: "gpt-4.1".to_string(),
                    name: Some("Duplicate".to_string()),
                    context_window: None,
                    max_output_tokens: None,
                    reasoning: None,
                    vision: None,
                    supports_attachments: None,
                    input_cost_per_million: None,
                    output_cost_per_million: None,
                    cached_input_cost_per_million: None,
                    cached_output_cost_per_million: None,
                    source: ModelSource::LocalObject,
                },
            ],
        };

        let entry = render_crush_provider_entry("target", &managed);
        let models = entry["models"]
            .as_array()
            .expect("models should be present for provider entry");
        assert_eq!(models.len(), 2);
        assert_eq!(
            models,
            &vec![
                json!({
                    "id": "gpt-4.1",
                    "name": "GPT-4.1"
                }),
                json!({
                    "id": "gpt-4.1-mini",
                    "name": "gpt-4.1-mini"
                })
            ]
        );
    }

}
