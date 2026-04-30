use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};

use clap::CommandFactory;
use clap::builder::StyledStr;
use clap_complete::{CompleteEnv, CompletionCandidate};

use crate::cli::{Cli, CompletionShell};
use crate::config::{ProviderConfig, load_config};
use crate::protocol::Protocol;

const COMPLETE_ENV: &str = "AGENT_RUN_COMPLETE";

pub fn enable_dynamic_completion() {
    CompleteEnv::with_factory(crate::cli::build_cli)
        .var(COMPLETE_ENV)
        .complete();
}

pub fn print_completion(shell: CompletionShell) {
    let shell = match shell {
        CompletionShell::Bash => "bash",
        CompletionShell::Zsh => "zsh",
    };

    // SAFETY: completion setup runs during startup before any threads are spawned.
    unsafe {
        std::env::set_var(COMPLETE_ENV, shell);
    }

    let args = vec![command_name()];
    let current_dir = std::env::current_dir().ok();
    let result = CompleteEnv::with_factory(crate::cli::build_cli)
        .var(COMPLETE_ENV)
        .try_complete(args, current_dir.as_deref());

    // SAFETY: completion setup runs during startup before any threads are spawned.
    unsafe {
        std::env::remove_var(COMPLETE_ENV);
    }

    if let Err(err) = result {
        err.exit();
    }
}

pub fn provider_candidates() -> Vec<CompletionCandidate> {
    let Ok(config_path) = crate::config::config_path() else {
        return Vec::new();
    };
    let Ok(config) = load_config(&config_path) else {
        return Vec::new();
    };

    config
        .providers
        .iter()
        .map(|(provider, config)| {
            labeled_candidate(provider.clone(), &provider_help(config), 0)
                .id(Some(format!("provider:{provider}")))
        })
        .collect()
}

pub fn complete_models_for_current_provider(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(provider) = completion_provider_from_argv() else {
        return Vec::new();
    };
    let Ok(config_path) = crate::config::config_path() else {
        return Vec::new();
    };
    let Ok(config) = load_config(&config_path) else {
        return Vec::new();
    };
    let Some(provider) = config.providers.get(&provider) else {
        return Vec::new();
    };

    let current = current.to_string_lossy();
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();

    for model in model_candidates(provider) {
        if seen.insert(model.clone()) && model.starts_with(current.as_ref()) {
            candidates.push(
                labeled_candidate(
                    model.clone(),
                    &model_help(provider.default_model.as_deref(), &provider.models, &model),
                    0,
                )
                .id(Some(format!("model:{model}"))),
            );
        }
    }

    candidates
}

fn completion_provider_from_argv() -> Option<String> {
    let args = completion_words_from_env();
    let launch_index = args.iter().position(|arg| arg == "launch")?;
    let mut iter = args[launch_index + 1..].iter().peekable();

    while let Some(arg) = iter.next() {
        if arg.is_empty() {
            continue;
        }
        if arg == "--" {
            return None;
        }
        if arg == "--model" {
            let _ = iter.next();
            continue;
        }
        if arg.starts_with("--model=") {
            continue;
        }
        if arg == "--force" {
            if matches!(
                iter.peek().map(|next| next.as_str()),
                Some("model" | "protocol" | "all")
            ) {
                let _ = iter.next();
            }
            continue;
        }
        if arg.starts_with("--force=") || arg.starts_with('-') {
            continue;
        }

        return Some(arg.clone());
    }

    None
}

fn completion_words_from_env() -> Vec<String> {
    let mut args = std::env::args_os();
    let _program = args.next();
    let words = args.skip_while(|arg| arg != "--").skip(1);

    words
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn command_name() -> OsString {
    let cmd = Cli::command();
    cmd.get_bin_name().unwrap_or_else(|| cmd.get_name()).into()
}

fn labeled_candidate(value: impl Into<OsString>, label: &str, order: usize) -> CompletionCandidate {
    let label = StyledStr::from(label.to_owned());
    CompletionCandidate::new(value.into())
        .help(Some(label.clone()))
        .tag(Some(label))
        .display_order(Some(order))
}

fn provider_help(provider: &ProviderConfig) -> String {
    format!(
        "[{}{}{}] protocols: {}",
        if provider.protocols.contains(&Protocol::OpenaiResponses) {
            'r'
        } else {
            ' '
        },
        if provider
            .protocols
            .contains(&Protocol::OpenaiChatCompletions)
        {
            'c'
        } else {
            ' '
        },
        if provider.protocols.contains(&Protocol::Anthropic) {
            'a'
        } else {
            ' '
        },
        protocol_labels(&provider.protocols).join(" / ")
    )
}

fn protocol_labels(protocols: &[Protocol]) -> Vec<&'static str> {
    let mut labels = Vec::new();
    if protocols.contains(&Protocol::OpenaiResponses) {
        labels.push("responses");
    }
    if protocols.contains(&Protocol::OpenaiChatCompletions) {
        labels.push("chat");
    }
    if protocols.contains(&Protocol::Anthropic) {
        labels.push("anthropic");
    }
    labels
}

fn model_candidates(provider: &ProviderConfig) -> Vec<String> {
    let mut models = Vec::new();
    if let Some(default_model) = provider.default_model.as_ref() {
        models.push(default_model.clone());
    }
    for model in &provider.models {
        if !models.iter().any(|existing| existing == model) {
            models.push(model.clone());
        }
    }
    models
}

fn model_help(default_model: Option<&str>, configured_models: &[String], model: &str) -> String {
    let is_default = default_model == Some(model);
    let is_configured = configured_models
        .iter()
        .any(|configured| configured == model);

    match (is_default, is_configured) {
        (true, true) => "default + configured".to_string(),
        (true, false) => "default".to_string(),
        (false, true) => "configured".to_string(),
        (false, false) => "model".to_string(),
    }
}

// TODO(high): support cached remote model candidates for completion.
// TODO(high): add provider-scoped remote model cache invalidation commands.
// TODO(high): support candidate matching modes for very large remote model catalogs.
// TODO(high): compress remote model cache on disk.
