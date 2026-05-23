use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

const CACHE_TTL_SECS: u64 = 60;

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    output: String,
    env_snapshot: std::collections::BTreeMap<String, String>,
    timestamp: u64,
}

pub enum CacheResult {
    Hit(String),
    Miss,
}

fn cache_path(key: &str) -> PathBuf {
    let base = env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".cache")
        });
    base.join("claude-status").join(format!("{key}.json"))
}

fn current_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn cache_check(key: &str, env_depends: &[String], no_cache: bool) -> CacheResult {
    if no_cache {
        debug!("cache: {key} skipped (--no-cache)");
        return CacheResult::Miss;
    }

    let path = cache_path(key);
    if !path.is_file() {
        debug!("cache: {key} miss (no file)");
        return CacheResult::Miss;
    }

    let raw = match fs::read_to_string(&path) {
        Ok(r) => r,
        Err(e) => {
            warn!("cache: {key} read error: {e}");
            return CacheResult::Miss;
        }
    };

    let entry: CacheEntry = match serde_json::from_str(&raw) {
        Ok(e) => e,
        Err(e) => {
            warn!("cache: {key} parse error: {e}");
            return CacheResult::Miss;
        }
    };

    let age = current_epoch_secs().saturating_sub(entry.timestamp);
    if age > CACHE_TTL_SECS {
        debug!("cache: {key} expired (age={}s)", age);
        return CacheResult::Miss;
    }

    if !env_depends.is_empty() {
        for var in env_depends {
            let current = env::var(var).unwrap_or_default();
            let cached = entry.env_snapshot.get(var).map(|s| s.as_str()).unwrap_or("");
            if current != cached {
                debug!("cache: {key} env mismatch: {var} changed");
                return CacheResult::Miss;
            }
        }
    }

    debug!("cache: {key} hit (age={}s)", age);
    CacheResult::Hit(entry.output)
}

pub fn cache_write(key: &str, output: &str, env_depends: &[String]) {
    let path = cache_path(key);
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            warn!("cache: failed to create dir {}: {e}", parent.display());
            return;
        }
    }

    let mut env_snapshot = std::collections::BTreeMap::new();
    for var in env_depends {
        if let Ok(val) = env::var(var) {
            env_snapshot.insert(var.clone(), val);
        }
    }

    let entry = CacheEntry {
        output: output.to_string(),
        env_snapshot,
        timestamp: current_epoch_secs(),
    };

    match serde_json::to_string(&entry) {
        Ok(json) => {
            if let Err(e) = fs::write(&path, json) {
                warn!("cache: failed to write {}: {e}", path.display());
            }
        }
        Err(e) => warn!("cache: failed to serialize {key}: {e}"),
    }
}
