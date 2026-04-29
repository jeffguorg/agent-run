use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "agent-run")]
#[command(about = "Launch coding agents with temporary provider settings")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Config,
    Launch(LaunchArgs),
}

#[derive(Parser, Debug)]
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
    }
}
