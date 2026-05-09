use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),
    #[error("failed to read config from {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config from {path}: {source}")]
    ParseConfig {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to launch editor `{editor}`: {source}")]
    LaunchEditor {
        editor: String,
        #[source]
        source: std::io::Error,
    },
    #[error("editor `{editor}` exited with status {status}")]
    EditorFailed { editor: String, status: i32 },
    #[error("key_command must not be empty")]
    EmptyKeyCommand,
    #[error("failed to run key_command `{command}`: {source}")]
    RunKeyCommand {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("key_command `{command}` exited with status {status}")]
    KeyCommandFailed { command: String, status: i32 },
    #[error("key_command `{command}` returned an empty secret")]
    EmptyKeyOutput { command: String },
    #[error("http request to {url} failed: {source}")]
    Http {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("failed to parse models response from {url}: {reason}")]
    ModelsResponse { url: String, reason: String },
    #[error("failed to create temporary runtime directory: {0}")]
    TempDir(#[source] std::io::Error),
    #[error("failed to read cache file {path}: {source}")]
    ReadCache {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse cache file {path}: {source}")]
    ParseCache {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize cache file {path}: {source}")]
    SerializeCache {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write cache file {path}: {source}")]
    WriteCache {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write temporary config {path}: {source}")]
    WriteTempConfig {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to launch `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize TOML config for {path}: {source}")]
    SerializeTomlConfig {
        path: PathBuf,
        #[source]
        source: toml::ser::Error,
    },
}
