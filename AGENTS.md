# AGENTS.md

## Project Thesis

`agent-run` is a small Rust launcher that keeps provider config in one place and adapts it to different coding agents at launch time.

The main rule of the project is:

- keep provider definition centralized
- keep agent-specific adaptation local to each launcher
- prefer temporary runtime config over mutating long-lived user config

The only intentional exception is the one-time Claude onboarding state fix.

## Codemap

```text
src/main.rs
  CLI entrypoint and launch orchestration
  chooses the final protocol for the selected agent/provider pair

src/cli.rs
  clap definitions
  supported agents and force flags

src/config.rs
  app config loading
  XDG/home path resolution
  source config lookup for downstream agents

src/protocol.rs
  protocol enum
  key resolution
  model discovery and model selection

src/agent.rs
  agent-specific launch adapters
  runtime config generation and merge logic

src/error.rs
  shared application error type
```

Launch flow:

```text
parse CLI
-> load provider config
-> negotiate protocol
-> resolve key
-> discover/merge models
-> resolve final model
-> launch selected agent adapter
```

## Working Rules

- Treat `providers.*` in `config.yaml` as the source of truth.
- Treat `protocols` as provider capabilities, not agent identity.
- Keep protocol negotiation in `main.rs`; keep agent-specific config rendering in `agent.rs`.
- Do not rewrite user-owned long-lived config in place. Generate runtime config under cache and point the downstream tool at it.
- If an agent cannot be given a temporary per-process model/provider override by env, CLI flag, runtime config, or isolated home/data dir, do not support it.
- Preserve existing downstream config by merging, unless a deliberate replacement is required.
- Keep secrets out of committed files. Runtime injection is preferred.

## Agent Model

- `claude` requires `anthropic`
- `codex` requires `openai-responses`
- `hermes` requires `openai-chat-completions`
- `crush` prefers `openai-chat-completions`, then falls back to `anthropic`

If a new agent can speak more than one protocol, add it as an ordered preference list, not a single hard-coded protocol.

## Runtime Strategy

- Claude: env-driven launch, plus one-time onboarding state repair.
- Codex: generate runtime `config.toml` and isolated `CODEX_HOME`.
- Hermes: generate runtime `config.yaml` and isolated `HERMES_HOME`.
- Crush: generate runtime `crush.json`, pass `CRUSH_GLOBAL_CONFIG` as a config directory, and isolate `--data-dir`.

## Implementation Guidance

When adding or changing an agent:

1. Define its protocol requirements or preference order.
2. Reuse existing protocol/base URL/key/model resolution when possible.
3. Add a dedicated runtime config renderer only where the target tool truly needs one.
4. Keep cache layout and env overrides explicit and easy to inspect.
5. If the target tool cannot isolate model/provider selection to the launched process, drop support instead of mutating persistent user state.
6. Update `README.md` and `config.demo.yaml` if user-facing behavior changes.

When changing protocol handling:

1. Keep `Protocol` generic and wire-level.
2. Avoid provider-specific branching in `main.rs` beyond protocol selection.
3. Push tool-specific mapping details into the relevant launcher.

## Constraints

- `key_command` is argv-style, not shell syntax.
- Model discovery is synchronous and can fail on network/provider issues.
- Cache paths are sanitized from provider names.
- Deep merge helpers replace non-object subtrees wholesale when types differ.

## Validation

Use these after changes:

```bash
cargo fmt
cargo check
cargo clippy -- -D warnings
```
