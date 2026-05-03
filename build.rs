use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use time::OffsetDateTime;

fn main() {
    println!("cargo:rerun-if-env-changed=BUILD_GIT_HASH");
    println!("cargo:rerun-if-env-changed=BUILD_GIT_DIRTY");
    println!("cargo:rerun-if-env-changed=BUILD_GIT_DATE");
    println!("cargo:rerun-if-env-changed=BUILD_DATE");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
    println!("cargo:rerun-if-changed=.git/packed-refs");
    println!("cargo:rerun-if-changed=.git/index");

    let full_hash = std::env::var("BUILD_GIT_HASH")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| git(&["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());

    let short_hash = if full_hash == "unknown" {
        "unknown".to_string()
    } else {
        full_hash.chars().take(7).collect()
    };

    let dirty = parse_bool_env("BUILD_GIT_DIRTY").unwrap_or_else(|| {
        Path::new(".git").exists()
            && git(&["status", "--porcelain"])
                .map(|out| !out.is_empty())
                .unwrap_or(false)
    });

    let git_date = std::env::var("BUILD_GIT_DATE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| git(&["log", "-1", "--format=%cs"]))
        .unwrap_or_default();

    let build_date = std::env::var("BUILD_DATE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("SOURCE_DATE_EPOCH")
                .ok()
                .and_then(|s| s.parse::<i64>().ok())
                .map(format_iso_utc)
        })
        .unwrap_or_else(|| {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            format_iso_utc(secs)
        });

    let version_tag = if full_hash == "unknown" {
        "unknown".to_string()
    } else if dirty {
        format!("{short_hash}-dirty")
    } else {
        short_hash.clone()
    };

    println!("cargo:rustc-env=BUILD_GIT_HASH={short_hash}");
    println!("cargo:rustc-env=BUILD_GIT_VERSION_TAG={version_tag}");
    println!(
        "cargo:rustc-env=BUILD_GIT_DIRTY={}",
        if dirty { "true" } else { "false" }
    );
    println!("cargo:rustc-env=BUILD_GIT_DATE={git_date}");
    println!("cargo:rustc-env=BUILD_DATE={build_date}");
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    Some(text.trim().to_string())
}

fn parse_bool_env(name: &str) -> Option<bool> {
    match std::env::var(name)
        .ok()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn format_iso_utc(epoch: i64) -> String {
    let dt = OffsetDateTime::from_unix_timestamp(epoch).unwrap_or(OffsetDateTime::UNIX_EPOCH);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        dt.year(),
        dt.month() as u8,
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    )
}
