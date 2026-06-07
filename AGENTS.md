# AGENTS.md

## Project Thesis

`agent-run` is a small Rust launcher that keeps provider config in one place and adapts it to different coding agents at launch time.

The main rule of the project is:

- keep provider definition centralized
- keep agent-specific adaptation local to each launcher
- prefer temporary runtime config over mutating long-lived user config, except for tools whose native model is a multi-provider global config

The intentional exceptions are:

- the one-time Claude onboarding state fix
- Crush provider synchronization, because Crush is a multi-provider agent with dynamic model/provider switching

## Working Rules

- Treat `providers.*` in `config.yaml` as the source of truth.
- Treat `protocols` as provider capabilities, not agent identity.
- Do not rewrite user-owned long-lived config in place for single-provider launch adapters. Generate runtime config under cache and point the downstream tool at it.
- For multi-provider agents like Crush, preserve unmanaged user config, mark agent-run-managed entries explicitly, and keep real secrets injected at runtime.
- If an agent cannot be given a temporary per-process model/provider override by env, CLI flag, runtime config, or isolated home, do not support it.
- Preserve existing downstream config by merging, unless a deliberate replacement is required.
- Keep secrets out of committed files. Runtime injection is preferred.
- Update `README.md` and `config.demo.yaml` if user-facing behavior changes.

## Implementation Guidance

When adding or changing an agent:

1. Define its protocol requirements or preference order.
2. Reuse existing protocol/base URL/key/model resolution when possible.
3. Add a dedicated runtime config renderer only where the target tool truly needs one.
4. Keep cache layout and env overrides explicit and easy to inspect.
5. If the target tool cannot isolate model/provider selection to the launched process, drop support instead of mutating persistent user state.

When changing protocol handling:

1. Keep `Protocol` generic and wire-level.
2. Avoid provider-specific branching in `main.rs` beyond protocol selection.
3. Push tool-specific mapping details into the relevant launcher.

## Constraints

- `key_command` is argv-style, not shell syntax.
- Model discovery is synchronous and can fail on network/provider issues.
- Cache paths are sanitized from provider names.
