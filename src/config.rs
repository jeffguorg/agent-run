use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde::Deserialize;

use crate::error::AppError;
use crate::protocol::Protocol;

const DEMO_CONFIG: &str = include_str!("../config.demo.yaml");

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub providers: BTreeMap<String, ProviderConfig>,
}

#[derive(Debug, Deserialize)]
pub struct ProviderConfig {
    pub protocols: Vec<Protocol>,
    pub base_urls: BaseUrls,
    pub key: Option<String>,
    pub key_command: Option<Vec<String>>,
    #[serde(default)]
    pub anthropic_use_api_key: bool,
    pub default_model: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub disable_model_loading_from_api: bool,
}

#[derive(Debug, Deserialize)]
pub struct BaseUrls {
    pub openai: Option<String>,
    pub anthropic: Option<String>,
}

pub fn load_config(path: &Path) -> Result<AppConfig, AppError> {
    if !path.exists() {
        return Err(AppError::Message(format!(
            "config file does not exist: {}",
            path.display()
        )));
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

pub fn run_config(bootstrap_config: bool) -> Result<ExitCode, AppError> {
    let path = config_path()?;
    if !path.exists() {
        if !bootstrap_config {
            return Err(AppError::Message(format!(
                "config file does not exist: {}; rerun with `agent-run config --bootstrap-config` to create a sample config",
                path.display()
            )));
        }

        eprintln!(
            "warning: config file does not exist at {}; writing embedded sample config",
            path.display()
        );
        write_demo_config(&path)?;
    }

    let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let status = Command::new(&editor)
        .arg(&path)
        .status()
        .map_err(|source| AppError::LaunchEditor {
            editor: editor.clone(),
            source,
        })?;

    if status.success() {
        Ok(ExitCode::SUCCESS)
    } else {
        Err(AppError::EditorFailed {
            editor,
            status: status.code().unwrap_or(-1),
        })
    }
}

pub fn config_path() -> Result<PathBuf, AppError> {
    if let Ok(path) = env::var("AGENT_RUN_CONFIG") {
        return Ok(PathBuf::from(path));
    }

    if let Ok(xdg_config_home) = env::var("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(xdg_config_home)
            .join("agent-run")
            .join("config.yaml"));
    }

    let home = env::var("HOME").map_err(|_| {
        AppError::Message("cannot resolve config path: HOME is not set".to_string())
    })?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("agent-run")
        .join("config.yaml"))
}

pub fn cache_dir() -> Result<PathBuf, AppError> {
    if let Ok(path) = env::var("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(path));
    }

    let home = env::var("HOME")
        .map_err(|_| AppError::Message("cannot resolve cache path: HOME is not set".to_string()))?;
    Ok(PathBuf::from(home).join(".cache"))
}

pub fn source_codex_config_path() -> Result<PathBuf, AppError> {
    if let Ok(codex_home) = env::var("CODEX_HOME") {
        return Ok(PathBuf::from(codex_home).join("config.toml"));
    }

    let home = env::var("HOME").map_err(|_| {
        AppError::Message("cannot resolve Codex config path: HOME is not set".to_string())
    })?;
    Ok(PathBuf::from(home).join(".codex").join("config.toml"))
}

pub fn source_crush_config_path() -> Result<PathBuf, AppError> {
    if let Ok(path) = env::var("CRUSH_GLOBAL_CONFIG") {
        return Ok(PathBuf::from(path).join("crush.json"));
    }

    let home = env::var("HOME").map_err(|_| {
        AppError::Message("cannot resolve Crush config path: HOME is not set".to_string())
    })?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("crush")
        .join("crush.json"))
}

pub fn source_hermes_home() -> Result<PathBuf, AppError> {
    if let Ok(path) = env::var("HERMES_HOME") {
        return Ok(PathBuf::from(path));
    }

    let home = env::var("HOME").map_err(|_| {
        AppError::Message("cannot resolve Hermes home path: HOME is not set".to_string())
    })?;
    Ok(PathBuf::from(home).join(".hermes"))
}

pub fn source_hermes_config_path() -> Result<PathBuf, AppError> {
    Ok(source_hermes_home()?.join("config.yaml"))
}

pub fn source_claude_state_path() -> Result<PathBuf, AppError> {
    let home = env::var("HOME").map_err(|_| {
        AppError::Message("cannot resolve Claude state path: HOME is not set".to_string())
    })?;
    Ok(PathBuf::from(home).join(".claude.json"))
}

pub fn claude_onboarding_lock_path() -> Result<PathBuf, AppError> {
    Ok(cache_dir()?
        .join("agent-run")
        .join("anthropic-onboarding-completed.lock"))
}

pub fn try_create_lock_file(path: &Path) -> Result<bool, AppError> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(AppError::TempDir)?;
    }
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(AppError::Message(format!(
                "onboarding lock creation raced at {}; retry the command",
                path.display()
            )))
        }
        Err(err) => Err(AppError::TempDir(err)),
    }
}

fn ensure_parent_dir(path: &Path) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            AppError::Message(format!(
                "failed to create config directory {}: {source}",
                parent.display()
            ))
        })?;
    }
    Ok(())
}

fn write_demo_config(path: &Path) -> Result<(), AppError> {
    ensure_parent_dir(path)?;
    fs::write(path, DEMO_CONFIG).map_err(|source| {
        AppError::Message(format!(
            "failed to initialize config at {}: {source}",
            path.display()
        ))
    })
}
