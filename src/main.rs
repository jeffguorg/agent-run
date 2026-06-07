mod agent;
mod catalog;
mod cli;
mod completion;
mod config;
mod error;
mod hook;
mod model;
mod model_lua;
mod protocol;
mod statusline;

use std::collections::{BTreeMap, BTreeSet};
use std::process::ExitCode;

use agent::{
    ManagedProviderLaunch, ResolvedLaunch, collect_shell_env, launch, preferred_protocols,
    shell_escape,
};
use catalog::{RemoteLoadMode, collect_provider_model_catalog, load_remote_models_for_protocol};
use cli::{ClaudeCodeHookCommands, Commands, ForceScopeSet, ModelsCommands, agent_name};
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
        Commands::ClaudeCodeHook(args) => match args.command {
            ClaudeCodeHookCommands::StopFailure(args) => hook::run_stop_failure(
                args.dry_run,
                args.unknown_error_rewake_in_secs,
                args.recheck_interval_seconds,
            ),
        },
        Commands::Launch(args) => {
            let force = ForceScopeSet::from_scope(args.force);
            let config_path = config_path()?;
            let config = load_config(&config_path)?;
            let target_pcfg = config.providers.get(&args.provider);
            let isolated = has_isolated_home(&config, args.agent, &args.provider);

            let (providers, configured_providers, target_model) = if args.agent.needs_all_providers() {
                resolve_all_providers_launch(
                    &config,
                    &args.provider,
                    args.agent,
                    args.model.as_deref(),
                    force,
                )?
            } else {
                // --- Single-provider path (claude, codex, hermes, shell) ---
                let managed = target_pcfg
                    .map(|pcfg| resolve_provider_launch(&args.provider, pcfg, args.agent, force))
                    .transpose();
                let managed = match managed {
                    Ok(m) => m,
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

                if managed.is_none() && !isolated {
                    return Err(missing_launch_target_error(
                        &config,
                        &args.provider,
                        args.agent,
                    ));
                }

                let target_model = match managed.as_ref() {
                    Some(m) => Some(resolve_target_model(
                        &args.provider,
                        m,
                        args.model.as_deref(),
                        force,
                    )?),
                    None => None,
                };

                let providers = managed
                    .map(|m| {
                        let mut map = BTreeMap::new();
                        map.insert(args.provider.clone(), m);
                        map
                    })
                    .unwrap_or_default();
                let configured_providers: BTreeSet<String> =
                    providers.keys().cloned().collect();

                (providers, configured_providers, target_model)
            };

            let launch_spec = ResolvedLaunch {
                agent: args.agent,
                target_provider: &args.provider,
                target_model,
                configured_providers,
                providers,
                agent_args: args.agent_args,
            };

            launch(args.agent, launch_spec)
        }
        Commands::ExportEnv(args) => run_export_env(args.provider, args.model),
    }
}

fn resolve_all_providers_launch<'a>(
    config: &'a AppConfig,
    target_provider_name: &str,
    agent: cli::Agent,
    requested_model: Option<&str>,
    force: ForceScopeSet,
) -> Result<
    (
        BTreeMap<String, ManagedProviderLaunch<'a>>,
        BTreeSet<String>,
        Option<String>,
    ),
    AppError,
> {
    let Some(target_pcfg) = config.providers.get(target_provider_name) else {
        return Err(missing_launch_target_error(
            config,
            target_provider_name,
            agent,
        ));
    };

    let target_managed = resolve_provider_launch(target_provider_name, target_pcfg, agent, force)?;
    let target_model = resolve_target_model(
        target_provider_name,
        &target_managed,
        requested_model,
        force,
    )?;
    let configured_providers: BTreeSet<String> = config.providers.keys().cloned().collect();
    let mut providers = BTreeMap::new();
    providers.insert(target_provider_name.to_string(), target_managed);
    for (name, pcfg) in &config.providers {
        if name == target_provider_name {
            continue;
        }
        match resolve_provider_launch(name, pcfg, agent, force) {
            Ok(m) => {
                providers.insert(name.clone(), m);
            }
            Err(e) => {
                warn!(
                    provider = %name,
                    error = %e,
                    "skipping provider"
                );
            }
        }
    }

    Ok((providers, configured_providers, Some(target_model)))
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
        cli::Agent::Claude | cli::Agent::Crush | cli::Agent::Shell => false,
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
    let hint = if supports_isolated_home(agent) {
        format!("; define `providers.{name}` or `isolated_homes.{isolated_key}.{name}`")
    } else {
        format!("; define `providers.{name}`")
    };
    AppError::Message(format!(
        "unknown launch target `{name}` for agent `{isolated_key}`{hint}"
    ))
}

fn resolve_provider_launch<'a>(
    provider_name: &str,
    provider: &'a ProviderConfig,
    agent: cli::Agent,
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
    let models = merge_models(&local_models, &remote_models.models);
    info!(
        provider = provider_name,
        protocol = protocol_name(protocol),
        used_cache = remote_models.used_cache,
        merged_model_count = models.len(),
        "resolved provider"
    );

    Ok(ManagedProviderLaunch {
        provider,
        protocol,
        key,
        models,
    })
}

fn resolve_target_model(
    provider_name: &str,
    managed: &ManagedProviderLaunch<'_>,
    requested_model: Option<&str>,
    force: ForceScopeSet,
) -> Result<String, AppError> {
    let merged_model_ids = model_ids(&managed.models);
    let selected_model =
        resolve_model(managed.provider, &merged_model_ids, requested_model, force)?;
    info!(
        provider = provider_name,
        protocol = protocol_name(managed.protocol),
        selected_model,
        "resolved launch model"
    );
    Ok(selected_model)
}

fn run_export_env(
    provider_name: String,
    requested_model: Option<String>,
) -> Result<ExitCode, AppError> {
    let config_path = config_path()?;
    let config = load_config(&config_path)?;
    let provider = config
        .providers
        .get(&provider_name)
        .ok_or_else(|| AppError::Message(format!("unknown provider `{provider_name}`")))?;
    let managed = resolve_provider_launch(
        &provider_name,
        provider,
        cli::Agent::Shell,
        ForceScopeSet::default(),
    )?;
    let target_model = resolve_target_model(
        &provider_name,
        &managed,
        requested_model.as_deref(),
        ForceScopeSet::default(),
    )?;
    let mut providers = BTreeMap::new();
    providers.insert(provider_name.clone(), managed);
    let launch = ResolvedLaunch {
        agent: cli::Agent::Shell,
        target_provider: &provider_name,
        target_model: Some(target_model),
        configured_providers: [provider_name.clone()].into_iter().collect(),
        providers,
        agent_args: vec![],
    };
    for (key, value) in collect_shell_env(&launch)? {
        println!("export {key}={}", shell_escape(&value));
    }
    Ok(ExitCode::SUCCESS)
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
            "{}\treasoning={}\tvision={}\tattachments={}\tcontext_window={}\tmax_output_tokens={}\tinput_cost_per_million={}\toutput_cost_per_million={}\tname={}",
            model.id,
            model.effective_reasoning(),
            model.effective_vision(),
            model.supports_attachments.unwrap_or(false),
            model
                .context_window
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            model
                .max_output_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            model
                .input_cost_per_million
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            model
                .output_cost_per_million
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            model.name.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::cli::Agent;
    use crate::config::{AppConfig, BaseUrls, IsolatedHomesConfig, ProviderConfig};
    use crate::model::{ModelApiFilterConfig, RawModelConfig};
    use crate::protocol::Protocol;

    fn provider(default_model: Option<&str>, key: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            protocols: vec![Protocol::OpenaiChatCompletions],
            base_urls: BaseUrls {
                openai: Some("https://example.test/v1".to_string()),
                anthropic: None,
            },
            key: key.map(ToOwned::to_owned),
            key_command: None,
            anthropic_use_api_key: false,
            default_model: default_model.map(ToOwned::to_owned),
            models: vec![RawModelConfig::String("gpt-4.1".to_string())],
            model_api_filters: ModelApiFilterConfig::Disabled,
            legacy_disable_model_loading_from_api: None,
            extra_env: BTreeMap::new(),
        }
    }

    #[test]
    fn all_providers_launch_resolves_target_default_model_for_crush_context() {
        let mut providers = BTreeMap::new();
        providers.insert(
            "target".to_string(),
            provider(Some("gpt-4.1"), Some("target-key")),
        );
        providers.insert("other".to_string(), provider(Some("gpt-4.1"), None));
        let config = AppConfig {
            providers,
            isolated_homes: IsolatedHomesConfig::default(),
        };

        let (_providers, _configured_providers, target_model) = resolve_all_providers_launch(
            &config,
            "target",
            Agent::Crush,
            None,
            ForceScopeSet::default(),
        )
        .expect("target provider should resolve");

        assert_eq!(target_model.as_deref(), Some("gpt-4.1"));
    }
}
