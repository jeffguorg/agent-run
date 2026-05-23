use std::env;

#[derive(Debug, Default)]
pub struct GitInfo {
    pub branch: Option<String>,
    pub dirty: bool,
}

pub fn git_info(cwd: &str) -> GitInfo {
    let mut info = GitInfo::default();

    let output = std::process::Command::new("git")
        .args(["-C", cwd, "rev-parse", "--abbrev-ref", "HEAD"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
            info.branch = if branch.is_empty() {
                None
            } else {
                Some(branch)
            };
        }
        _ => return info,
    }

    let output2 = std::process::Command::new("git")
        .args(["-C", cwd, "status", "--porcelain"])
        .output();

    if let Ok(out) = output2 {
        if out.status.success() {
            info.dirty = !String::from_utf8_lossy(&out.stdout).trim().is_empty();
        }
    }

    info
}

pub fn short_path(input: &str) -> String {
    let home = env::var("HOME").unwrap_or_default();
    let home_str = if home.ends_with('/') {
        &home[..home.len() - 1]
    } else {
        &home
    };

    if input == home_str {
        return "~".to_string();
    }

    let p = if let Some(stripped) = input.strip_prefix(home_str) {
        if stripped.starts_with('/') {
            format!("~{stripped}")
        } else {
            input.to_string()
        }
    } else {
        input.to_string()
    };

    let separator = "/";
    let parts: Vec<&str> = p.split(separator).collect();
    if parts.len() <= 1 {
        return p;
    }

    let mut head: Vec<String> = parts[..parts.len() - 1]
        .iter()
        .map(|s: &&str| {
            if s.len() > 6 {
                format!("{}...", &s[..3])
            } else {
                (*s).to_string()
            }
        })
        .collect();
    head.push(parts.last().unwrap().to_string());
    head.join("/")
}
