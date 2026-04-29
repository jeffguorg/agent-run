# agent-run

`agent-run` is a small launcher for coding agents.

It keeps provider settings in one place, then starts a target agent with temporary runtime config instead of asking you to manually rewrite per-agent config files every time.

Current focus:

- `claude`
- `codex`
- `hermes`

Current protocol support:

- `anthropic`
- `openai-responses`
- `openai-chat-completions`

## Why

Different coding agents expect different configuration shapes:

- some read environment variables
- some want a config file or profile
- some mix both

`agent-run` gives you one provider config and adapts it to the target agent at launch time.

## What It Does

- Centralizes provider definitions in one config file
- Resolves secrets from either `key` or `key_command`
- Optionally loads model lists from provider APIs
- Validates protocol compatibility before launch
- Generates temporary runtime config where needed
- Forwards extra args to the underlying agent command

## Supported Agents

### Claude Code

- protocol: `anthropic`
- launch mode: environment variables
- supports both `ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_API_KEY`
- ensures onboarding is marked complete before launch

### Codex

- protocol: `openai-responses`
- launch mode: temporary `CODEX_HOME` and generated `config.toml`
- merges existing Codex config into the runtime config

### Hermes Agent

- protocol: `openai-chat-completions`
- launch mode: temporary `HERMES_HOME` and generated `config.yaml`
- injects API key via environment variable

## Configuration

Default config path:

```text
~/.config/agent-run/config.yaml
```

Minimal example:

```yaml
providers:
  deepseek:
    protocols:
      - openai-chat-completions
      - anthropic
    base_urls:
      openai: https://api.deepseek.com
      anthropic: https://api.deepseek.com/anthropic
    key_command:
      - printenv
      - DEEPSEEK_API_KEY
    default_model: deepseek-v4-pro
    models:
      - deepseek-v4-pro
    disable_model_loading_from_api: true

  kimi-code:
    protocols:
      - openai-chat-completions
      - anthropic
    base_urls:
      openai: https://api.kimi.com/coding/v1
      anthropic: https://api.kimi.com/coding
    key_command:
      - printenv
      - KIMI_API_KEY
    anthropic_use_api_key: true
    default_model: kimi-for-coding
    models:
      - kimi-for-coding
    disable_model_loading_from_api: true
```

See [config.demo.yaml](config.demo.yaml) for a fuller example.

## Usage

Open or initialize your config:

```bash
agent-run config
```

Launch Claude:

```bash
agent-run launch deepseek claude
agent-run launch kimi-code claude
```

Launch Codex:

```bash
agent-run launch ollama codex
agent-run launch openrouter --model openai/gpt-5.3-codex codex
```

Launch Hermes:

```bash
agent-run launch deepseek hermes
agent-run launch ollama hermes
```

Forward extra args to the underlying agent:

```bash
agent-run launch deepseek claude resume
agent-run launch ollama codex resume --last
agent-run launch deepseek hermes -- chat -q "hello"
```

Both forms are supported:

- `agent-run launch provider agent arg1 arg2`
- `agent-run launch provider agent -- arg1 arg2`

## Secret Handling

Each provider can use either:

- `key`
- `key_command`

`key_command` is preferred when you want to fetch secrets from an external source such as a password manager, shell environment, or local secret helper.

## Runtime Strategy

`agent-run` tries to avoid modifying long-lived agent config unless necessary.

- `claude` uses temporary env plus a one-time onboarding state fix
- `codex` uses generated runtime config under cache
- `hermes` uses generated runtime config under cache

## Development

This repository uses a minimal Rust + Nix setup.

Useful commands:

```bash
cargo check
cargo clippy -- -D warnings
cargo fmt
```

## Docs

- [Project Notes](docs/INVESTMENT.md)
- [Claude Code](docs/tools/claude-code.md)
- [Codex](docs/tools/codex.md)
- [Hermes Agent](docs/tools/hermes-agent.md)
