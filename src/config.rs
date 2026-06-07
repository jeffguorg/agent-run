use std::borrow::Cow;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde::Deserialize;

use crate::error::AppError;
use crate::model::{ModelApiFilterConfig, RawModelConfig};
use crate::protocol::Protocol;

#[cfg(unix)]
mod unix_platform {
    pub const HOME_ENV: &str = "HOME";
    pub const CACHE_HOME_ENV: &str = "XDG_CACHE_HOME";
    pub const DEFAULT_EDITOR: &str = "vi";
}

#[cfg(windows)]
mod windows_platform {
    pub const HOME_ENV: &str = "USERPROFILE";
    pub const CACHE_HOME_ENV: &str = "TEMP";
    pub const DEFAULT_EDITOR: &str = "notepad.exe";
}

#[cfg(unix)]
use unix_platform as current_platform;
#[cfg(windows)]
use windows_platform as current_platform;

const DEMO_CONFIG: &str = include_str!("../config.demo.yaml");

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default, alias = "isolated-homes")]
    pub isolated_homes: IsolatedHomesConfig,
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
    pub models: Vec<RawModelConfig>,
    #[serde(default)]
    pub model_api_filters: ModelApiFilterConfig,
    #[serde(default, rename = "disable_model_loading_from_api")]
    pub legacy_disable_model_loading_from_api: Option<bool>,
    #[serde(default)]
    pub extra_env: BTreeMap<String, String>,
}

impl ProviderConfig {
    pub fn effective_model_api_filters(&self) -> Cow<'_, ModelApiFilterConfig> {
        if self.legacy_disable_model_loading_from_api == Some(true) {
            Cow::Owned(ModelApiFilterConfig::Disabled)
        } else {
            Cow::Borrowed(&self.model_api_filters)
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct IsolatedHomesConfig {
    #[serde(default)]
    pub codex: BTreeMap<String, IsolatedHomeConfig>,
    #[serde(default)]
    pub hermes: BTreeMap<String, IsolatedHomeConfig>,
}

#[derive(Debug, Default, Deserialize)]
pub struct IsolatedHomeConfig {}

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

    let editor =
        env::var("EDITOR").unwrap_or_else(|_| current_platform::DEFAULT_EDITOR.to_string());
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

    let home = env::var(current_platform::HOME_ENV).map_err(|_| {
        AppError::Message(format!(
            "cannot resolve config path: {} is not set",
            current_platform::HOME_ENV
        ))
    })?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("agent-run")
        .join("config.yaml"))
}

pub fn cache_dir() -> Result<PathBuf, AppError> {
    if let Ok(path) = env::var(current_platform::CACHE_HOME_ENV) {
        return Ok(PathBuf::from(path));
    }

    #[cfg(unix)]
    {
        let home = env::var(current_platform::HOME_ENV).map_err(|_| {
            AppError::Message(format!(
                "cannot resolve cache path: {} is not set",
                current_platform::HOME_ENV
            ))
        })?;
        Ok(PathBuf::from(home).join(".cache"))
    }

    #[cfg(windows)]
    {
        Err(AppError::Message(format!(
            "cannot resolve cache path: {} is not set",
            current_platform::CACHE_HOME_ENV
        )))
    }
}

pub fn source_crush_config_path() -> Result<PathBuf, AppError> {
    if let Ok(path) = env::var("CRUSH_GLOBAL_CONFIG") {
        return Ok(PathBuf::from(path).join("crush.json"));
    }

    let home = env::var(current_platform::HOME_ENV).map_err(|_| {
        AppError::Message(format!(
            "cannot resolve Crush config path: {} is not set",
            current_platform::HOME_ENV
        ))
    })?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("crush")
        .join("crush.json"))
}

pub fn model_script_dir() -> Result<PathBuf, AppError> {
    Ok(config_path()?
        .parent()
        .ok_or_else(|| AppError::Message("cannot resolve model script directory".to_string()))?
        .join("model.d"))
}

pub fn source_claude_state_path() -> Result<PathBuf, AppError> {
    let home = env::var(current_platform::HOME_ENV).map_err(|_| {
        AppError::Message(format!(
            "cannot resolve Claude state path: {} is not set",
            current_platform::HOME_ENV
        ))
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
