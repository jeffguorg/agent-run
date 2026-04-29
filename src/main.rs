mod agent;
mod cli;
mod config;
mod error;
mod protocol;

use std::process::ExitCode;

use clap::Parser;

use agent::{ResolvedLaunch, launch, required_protocol};
use cli::{Cli, Commands, ForceScopeSet, agent_name};
use config::{config_path, load_config, run_config};
use error::AppError;
use protocol::{
    base_url_for, fetch_models, merge_models, protocol_name, resolve_key, resolve_model,
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
    let cli = Cli::parse();
    match cli.command {
        Commands::Config => run_config(),
        Commands::Launch(args) => {
            let force = ForceScopeSet::from_scope(args.force);
            let config_path = config_path()?;
            let config = load_config(&config_path)?;
            let provider = config.providers.get(&args.provider).ok_or_else(|| {
                AppError::Message(format!("unknown provider `{}`", args.provider))
            })?;

            let protocol = required_protocol(args.agent);
            if !provider.protocols.contains(&protocol) && !force.protocol {
                return Err(AppError::Message(format!(
                    "provider `{}` does not support required protocol `{}` for agent `{}`",
                    args.provider,
                    protocol_name(protocol),
                    agent_name(args.agent)
                )));
            }

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
                key,
                model: selected_model,
                agent_args: args.agent_args,
            };

            launch(args.agent, launch_spec)
        }
    }
}
