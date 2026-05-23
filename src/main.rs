mod agent;
mod catalog;
mod cli;
mod completion;
mod config;
mod error;
mod model;
mod protocol;
mod statusline;

use std::process::ExitCode;

use agent::{ManagedProviderLaunch, ResolvedLaunch, launch, preferred_protocols};
use catalog::{RemoteLoadMode, collect_provider_model_catalog, load_remote_models_for_protocol};
use cli::{Commands, ForceScopeSet, ModelsCommands, agent_name};
use completion::{enable_dynamic_completion, print_completion};
use config::{AppConfig, ProviderConfig, config_path, load_config, run_config};
use error::AppError;
use model::{LoadRemoteModels, merge_models, model_ids, normalize_local_models};
use protocol::{Protocol, base_url_for, protocol_name, resolve_key, resolve_model};
use tracing::{debug, info, trace, warn};
use tracing_subscriber::EnvFilter;

const COMPLETE_ENV: &str = "AGENT_RUN_COMPLETE";

fn main() -> ExitCode {
    init_tracing();
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
        Commands::Config(args) => run_config(args.bootstrap_config),
        Commands::Completion(args) => {
            print_completion(args.shell);
            Ok(ExitCode::SUCCESS)
        }
        Commands::Models(args) => run_models_command(args.command),
        Commands::Version => {
            print_version();
            Ok(ExitCode::SUCCESS)
        }
        Commands::Statusline(args) => statusline::run_statusline(args.no_cache),
        Commands::Launch(args) => {
            let force = ForceScopeSet::from_scope(args.force);
            let config_path = config_path()?;
            let config = load_config(&config_path)?;
            let provider = config.providers.get(&args.provider);
            let isolated = has_isolated_home(&config, args.agent, &args.provider);
            let managed_provider = provider
                .map(|provider| {
                    resolve_managed_provider_launch(
                        &args.provider,
                        provider,
                        args.agent,
                        args.model.as_deref(),
                        force,
                    )
                })
                .transpose();
            let managed_provider = match managed_provider {
                Ok(launch) => launch,
                Err(err) if isolated && supports_isolated_home(args.agent) => {
                    warn!(
                        provider = %args.provider,
                        agent = agent_name(args.agent),
                        error = %err,
                        "provider launch resolution failed; falling back to isolated home"
                    );
                    None
                }
                Err(err) => return Err(err),
            };

            if managed_provider.is_none() && !isolated {
                return Err(missing_launch_target_error(
                    &config,
                    &args.provider,
                    args.agent,
                ));
            }

            let launch_spec = ResolvedLaunch {
                agent: args.agent,
                provider_name: &args.provider,
                managed_provider,
                agent_args: args.agent_args,
            };

            launch(args.agent, launch_spec)
        }
    }
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        if std::env::var_os(COMPLETE_ENV).is_some() {
            EnvFilter::new("off")
        } else {
            EnvFilter::new("warn")
        }
    });
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .init();
}

fn print_version() {
    println!("agent-run {}", env!("CARGO_PKG_VERSION"));
    println!("commit:     {}", env!("BUILD_GIT_HASH"));
    println!("dirty:      {}", env!("BUILD_GIT_DIRTY"));
    println!("commit-date:{}", env!("BUILD_GIT_DATE"));
    println!("build-date: {}", env!("BUILD_DATE"));
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

fn has_isolated_home(config: &AppConfig, agent: cli::Agent, name: &str) -> bool {
    match agent {
        cli::Agent::Codex => config.isolated_homes.codex.contains_key(name),
        cli::Agent::Hermes => config.isolated_homes.hermes.contains_key(name),
        cli::Agent::Claude | cli::Agent::Crush => false,
    }
}

fn supports_isolated_home(agent: cli::Agent) -> bool {
    matches!(agent, cli::Agent::Codex | cli::Agent::Hermes)
}

fn missing_launch_target_error(config: &AppConfig, name: &str, agent: cli::Agent) -> AppError {
    if config.providers.contains_key(name) {
        return AppError::Message(format!(
            "launch target `{name}` exists as a provider, but agent `{}` cannot start from it",
            agent_name(agent)
        ));
    }

    let isolated_key = agent_name(agent);
    AppError::Message(format!(
        "unknown launch target `{name}` for agent `{isolated_key}`; define `providers.{name}` or `isolated_homes.{isolated_key}.{name}`"
    ))
}

fn resolve_managed_provider_launch<'a>(
    provider_name: &str,
    provider: &'a ProviderConfig,
    agent: cli::Agent,
    requested_model: Option<&str>,
    force: ForceScopeSet,
) -> Result<ManagedProviderLaunch<'a>, AppError> {
    let protocol = resolve_protocol(agent, &provider.protocols, force)?;
    trace!(
        provider = provider_name,
        protocol = protocol_name(protocol),
        agent = agent_name(agent),
        "resolved provider protocol"
    );

    let base_url = base_url_for(provider, protocol).ok_or_else(|| {
        AppError::Message(format!(
            "provider `{provider_name}` is missing base URL for protocol `{}`",
            protocol_name(protocol)
        ))
    })?;

    let key = resolve_key(provider)?;
    let effective_filters = provider.effective_model_api_filters();
    let local_models = normalize_local_models(&provider.models);
    debug!(
        provider = provider_name,
        local_model_count = local_models.len(),
        model_api_filters_enabled = effective_filters.is_enabled(),
        "loaded local models from config"
    );
    let remote_models = if !effective_filters.is_enabled() {
        LoadRemoteModels::default()
    } else {
        load_remote_models_for_protocol(
            provider_name,
            protocol,
            base_url,
            &key,
            false,
            effective_filters.as_ref(),
        )?
    };
    let merged_models = merge_models(&local_models, &remote_models.models);
    let merged_model_ids = model_ids(&merged_models);
    let selected_model = resolve_model(provider, &merged_model_ids, requested_model, force)?;
    info!(
        provider = provider_name,
        protocol = protocol_name(protocol),
        selected_model,
        used_cache = remote_models.used_cache,
        merged_model_count = merged_models.len(),
        "resolved launch model"
    );

    Ok(ManagedProviderLaunch {
        provider,
        protocol,
        key,
        model: selected_model,
    })
}

fn run_models_command(command: ModelsCommands) -> Result<ExitCode, AppError> {
    match command {
        ModelsCommands::List(args) => {
            let config_path = config_path()?;
            let config = load_config(&config_path)?;
            let mode = if args.refresh {
                RemoteLoadMode::ForceRefresh
            } else {
                RemoteLoadMode::AutoRefresh
            };

            if args.all {
                for (index, (provider_name, provider)) in config.providers.iter().enumerate() {
                    if index > 0 {
                        println!();
                    }
                    print_provider_models(provider_name, provider, mode)?;
                }
            } else {
                let provider_name = args.provider.expect("clap requires provider or --all");
                let provider = config.providers.get(&provider_name).ok_or_else(|| {
                    AppError::Message(format!("unknown provider `{provider_name}`"))
                })?;
                print_provider_models(&provider_name, provider, mode)?;
            }

            Ok(ExitCode::SUCCESS)
        }
    }
}

fn print_provider_models(
    provider_name: &str,
    provider: &crate::config::ProviderConfig,
    mode: RemoteLoadMode,
) -> Result<(), AppError> {
    let catalog = collect_provider_model_catalog(provider_name, provider, mode)?;
    println!("{provider_name}");
    println!(
        "protocol: {}",
        catalog.protocol.map(protocol_name).unwrap_or("none")
    );
    println!(
        "source: {}",
        if catalog.used_cache {
            "cache"
        } else if !provider.effective_model_api_filters().is_enabled() {
            "local-only"
        } else {
            "remote"
        }
    );
    println!("models: {}", catalog.merged_models.len());
    for model in &catalog.merged_models {
        println!(
            "{}\treasoning={}\tvision={}\tcontext_window={}\tname={}",
            model.id,
            model.effective_reasoning(),
            model.effective_vision(),
            model
                .context_window
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            model.name.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}
