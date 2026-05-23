use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use clap_complete::{ArgValueCandidates, ArgValueCompleter};

use crate::completion::{complete_models_for_current_provider, provider_candidates};

#[derive(Parser, Debug)]
#[command(name = "agent-run")]
#[command(about = "Launch coding agents with temporary provider settings")]
#[command(version = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("BUILD_GIT_VERSION_TAG"),
    ")"
))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Config(ConfigArgs),
    Launch(LaunchArgs),
    Completion(CompletionArgs),
    Models(ModelsArgs),
    Version,
    Statusline(StatuslineArgs),
}

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[arg(long)]
    pub bootstrap_config: bool,
}

#[derive(Args, Debug)]
pub struct LaunchArgs {
    pub provider: String,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(
        long,
        value_enum,
        num_args = 0..=1,
        default_missing_value = "all",
        require_equals = false
    )]
    pub force: Option<ForceScope>,
    pub agent: Agent,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub agent_args: Vec<String>,
}

#[derive(Args, Debug)]
pub struct CompletionArgs {
    pub shell: CompletionShell,
}

#[derive(Args, Debug)]
pub struct ModelsArgs {
    #[command(subcommand)]
    pub command: ModelsCommands,
}

#[derive(Subcommand, Debug)]
pub enum ModelsCommands {
    List(ModelsListArgs),
}

#[derive(Args, Debug)]
pub struct ModelsListArgs {
    #[arg(long)]
    pub refresh: bool,
    #[arg(long, conflicts_with = "provider")]
    pub all: bool,
    #[arg(required_unless_present = "all")]
    pub provider: Option<String>,
}

#[derive(Args, Debug)]
pub struct StatuslineArgs {
    #[arg(long)]
    pub no_cache: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum ForceScope {
    Model,
    Protocol,
    All,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum Agent {
    Claude,
    Codex,
    Hermes,
    Crush,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    // TODO(low): support fish completion.
    // TODO(low): support nushell completion.
    // TODO(low): evaluate powershell and elvish support.
}

#[derive(Copy, Clone, Debug, Default)]
pub struct ForceScopeSet {
    pub model: bool,
    pub protocol: bool,
}

impl ForceScopeSet {
    pub fn from_scope(scope: Option<ForceScope>) -> Self {
        match scope {
            Some(ForceScope::Model) => Self {
                model: true,
                protocol: false,
            },
            Some(ForceScope::Protocol) => Self {
                model: false,
                protocol: true,
            },
            Some(ForceScope::All) => Self {
                model: true,
                protocol: true,
            },
            None => Self::default(),
        }
    }
}

pub fn agent_name(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        Agent::Hermes => "hermes",
        Agent::Crush => "crush",
    }
}

pub fn build_cli() -> clap::Command {
    Cli::command()
        .mut_subcommand("launch", |subcmd| {
            subcmd.mut_args(|arg| match arg.get_id().as_str() {
                "provider" => arg.add(ArgValueCandidates::new(provider_candidates)),
                "model" => arg.add(ArgValueCompleter::new(complete_models_for_current_provider)),
                _ => arg,
            })
        })
        .mut_subcommand("models", |subcmd| {
            subcmd.mut_subcommand("list", |subcmd| {
                subcmd.mut_args(|arg| match arg.get_id().as_str() {
                    "provider" => arg.add(ArgValueCandidates::new(provider_candidates)),
                    _ => arg,
                })
            })
        })
}

pub fn parse() -> Cli {
    let mut cmd = build_cli();
    let mut matches = cmd.get_matches_mut();
    Cli::from_arg_matches_mut(&mut matches).unwrap_or_else(|err| err.exit())
}
