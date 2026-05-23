use std::io::Read;
use std::process::ExitCode;

use tracing::debug;

use crate::error::AppError;

mod cache;
mod context;
mod lua_engine;

pub fn run_statusline(no_cache: bool) -> Result<ExitCode, AppError> {
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .map_err(|e| AppError::StatuslineStdin(e.to_string()))?;

    let stdin_data: serde_json::Value = if raw.trim().is_empty() {
        serde_json::Value::Object(Default::default())
    } else {
        serde_json::from_str(&raw).map_err(AppError::StatuslineJson)?
    };
    debug!("stdin keys: {:?}", stdin_data.as_object().map(|m| m.keys().collect::<Vec<_>>()));

    let output = lua_engine::run_parts(&stdin_data, no_cache)?;
    println!("{output}");
    Ok(ExitCode::SUCCESS)
}
