mod agent;
mod cli;
mod completion;
mod config;
mod error;
mod protocol;

use std::process::ExitCode;

use agent::{ResolvedLaunch, launch, preferred_protocols};
use cli::{Commands, ForceScopeSet, agent_name};
use completion::{enable_dynamic_completion, print_completion};
use config::{config_path, load_config, run_config};
use error::AppError;
use protocol::{
    Protocol, base_url_for, fetch_models, merge_models, protocol_name, resolve_key, resolve_model,
};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, AppError> {
    enable_dynamic_completion();
    let cli = cli::parse();
    match cli.command {
        Commands::Config => run_config(),
        Commands::Completion(args) => {
            print_completion(args.shell);
            Ok(ExitCode::SUCCESS)
        }
        Commands::Launch(args) => {
            let force = ForceScopeSet::from_scope(args.force);
            let config_path = config_path()?;
            let config = load_config(&config_path)?;
            let provider = config.providers.get(&args.provider).ok_or_else(|| {
                AppError::Message(format!("unknown provider `{}`", args.provider))
            })?;

            let protocol = resolve_protocol(args.agent, &provider.protocols, force)?;

            let base_url = base_url_for(provider, protocol).ok_or_else(|| {
                AppError::Message(format!(
                    "provider `{}` is missing base URL for protocol `{}`",
                    args.provider,
                    protocol_name(protocol)
                ))
            })?;

            let key = resolve_key(provider)?;
            let discovered_models = if provider.disable_model_loading_from_api {
                Vec::new()
            } else {
                fetch_models(protocol, base_url, &key)?
            };
            let merged_models = merge_models(&provider.models, &discovered_models);
            let selected_model =
                resolve_model(provider, &merged_models, args.model.as_deref(), force)?;

            let launch_spec = ResolvedLaunch {
                provider_name: &args.provider,
                provider,
                protocol,
                key,
                model: selected_model,
                agent_args: args.agent_args,
            };

            launch(args.agent, launch_spec)
        }
    }
}

fn resolve_protocol(
    agent: cli::Agent,
    provider_protocols: &[Protocol],
    force: ForceScopeSet,
) -> Result<Protocol, AppError> {
    let preferred = preferred_protocols(agent);
    if let Some(protocol) = preferred
        .iter()
        .copied()
        .find(|protocol| provider_protocols.contains(protocol))
    {
        return Ok(protocol);
    }

    if force.protocol {
        return Ok(preferred[0]);
    }

    let supported = preferred
        .iter()
        .map(|protocol| format!("`{}`", protocol_name(*protocol)))
        .collect::<Vec<_>>()
        .join(", ");
    Err(AppError::Message(format!(
        "provider does not support any protocol required by agent `{}`; supported choices are {}",
        agent_name(agent),
        supported
    )))
}
