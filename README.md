# agent-run

`agent-run` is a small launcher for coding agents.

It keeps provider settings in one place, then starts a target agent with temporary runtime config instead of asking you to manually rewrite per-agent config files every time.

Current focus:

- `claude`
- `codex`
- `hermes`
- `crush`

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

The provider config stays generic; each agent adapter is responsible for mapping that generic config into the shape the downstream tool expects.

## What It Does

- Centralizes provider definitions in one config file
- Resolves secrets from either `key` or `key_command`
- Optionally loads model lists from provider APIs and caches normalized results on disk
- Validates protocol compatibility before launch
- Negotiates the final protocol for agents that support more than one wire API
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
- does not read or merge any existing Codex home/config

### Hermes Agent

- protocol: `openai-chat-completions`
- launch mode: temporary `HERMES_HOME` and generated `config.yaml`
- injects API key via environment variable
- does not read or merge any existing Hermes home/config

### Crush

- protocol: prefers `openai-chat-completions`, falls back to `anthropic`
- launch mode: generated `crush.json` plus isolated `--data-dir`
- merges existing global Crush config into the runtime config

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
    model_api_filters: []

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

isolated_homes:
  codex:
    sandbox: {}
  hermes:
    sandbox: {}
```

See [config.demo.yaml](config.demo.yaml) for a fuller example.

## Protocol Negotiation

`protocols` declares what a provider can speak.

- `claude` requires `anthropic`
- `codex` requires `openai-responses`
- `hermes` requires `openai-chat-completions`
- `crush` prefers `openai-chat-completions` and falls back to `anthropic`

For single-protocol agents, launch fails unless the provider supports that protocol or `--force protocol` is used.
For `crush`, `agent-run` picks the first supported protocol from that preference order.

## Usage

Open or initialize your config:

```bash
agent-run config
agent-run config --bootstrap-config
```

`agent-run config` only opens an existing config. Use `--bootstrap-config` to write the embedded sample config first when the file does not exist.

Launch Claude:

```bash
agent-run launch deepseek claude
agent-run launch kimi-code claude
```

Launch Codex:

```bash
agent-run launch ollama codex
agent-run launch openrouter --model openai/gpt-5.3-codex codex
agent-run launch sandbox codex
```

Launch Hermes:

```bash
agent-run launch deepseek hermes
agent-run launch ollama hermes
agent-run launch sandbox hermes
```

`isolated_homes` notes:

- `isolated_homes.codex.<name>` and `isolated_homes.hermes.<name>` allow launching that agent with an isolated runtime home even when no provider exists.
- Entries are empty objects today. They do not accept `key`, `base_url`, `model`, or custom paths.
- When both a provider and an isolated home entry share the same name, agent-run combines them: isolated runtime home plus provider-derived runtime config.
- Runtime homes are rebuilt from scratch on each launch. agent-run never copies or merges your existing `CODEX_HOME` or `HERMES_HOME`.
- Optional skeleton files are copied from `~/.config/agent-run/skel/<agent>/` before agent-run writes any generated runtime config.

Launch Crush:

```bash
agent-run launch deepseek crush
agent-run launch kimi-code crush run "explain this repository"
```

Generate shell completion:

```bash
agent-run completion bash
agent-run completion zsh
```

When installed from Nix, bash and zsh completion files are installed automatically.

Manual shell setup is only needed when running the binary outside the Nix package:

```bash
source <(agent-run completion bash)
```

```zsh
source <(agent-run completion zsh)
```

Model catalog:

```bash
agent-run models list openrouter
agent-run models list --refresh openrouter
agent-run models list --all
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

Completion notes:

- Bash and Zsh are supported.
- Provider completion is loaded from local `config.yaml`.
- `--model` completion refreshes remote model cache by default when `model_api_filters` is enabled.
- Set `AGENT_RUN_DISABLE_MODEL_COMPLETION_REFRESH=1` to make completion use local models plus existing cache only.
- Default log level is `WARN`. Completion runs stay silent unless you explicitly set `RUST_LOG`.
- Trailing `agent_args` are forwarded but are not completed.

Model API filter notes:

- Omit `model_api_filters` to use the default catch-all rule.
- Set `model_api_filters: []` or `model_api_filters: null` to disable remote model loading and cache interaction for that provider.
- Filters are applied to normalized remote models before cache write.

## Secret Handling

Each provider can use either:

- `key`
- `key_command`

`key_command` is preferred when you want to fetch secrets from an external source such as a password manager, shell environment, or local secret helper.

## Extra Environment Variables

Each provider may declare an `extra_env` map that is injected into the launched agent process. The map is applied last and may override the env vars `agent-run` sets by default (e.g. `ANTHROPIC_API_KEY`, `CODEX_HOME`, `OPENAI_API_KEY`).

Values support inline template expansion:

- `${env:NAME}` &mdash; reads env var `NAME` from the launcher process; errors if unset.
- `${context:FIELD}` &mdash; reads a resolved launch field. Supported fields: `provider`, `protocol`, `model`, `key`, `agent`, `base_url`.

Example:

```yaml
providers:
  openrouter:
    # ...other fields...
    extra_env:
      OPENROUTER_API_KEY: "${context:key}"
      HTTPS_PROXY: "${env:CORP_PROXY}"
      AGENT_RUN_TRACE: "${context:agent}:${context:model}"
```

## Runtime Strategy

`agent-run` tries to avoid modifying long-lived agent config unless necessary.

- `claude` uses temporary env plus a one-time onboarding state fix
- `codex` uses generated runtime config under cache
- `hermes` uses generated runtime config under cache
- `crush` uses generated runtime config and isolated data dir under cache

Crush note:

- `CRUSH_GLOBAL_CONFIG` must point to a config directory, not directly to `crush.json`
- Crush may still read the current project directory and initialize project-local files such as `AGENTS.md`

## Development

This repository uses a minimal Rust + Nix setup.

Useful commands:

```bash
cargo check
cargo clippy -- -D warnings
cargo fmt
```

## Docs

- [Project Notes](docs/notes.md)
- [Claude Code](docs/tools/claude-code.md)
- [Codex](docs/tools/codex.md)
- [Crush](docs/tools/crush.md)
- [Hermes Agent](docs/tools/hermes-agent.md)
