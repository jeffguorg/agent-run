# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Tooling

- Run ad-hoc Python through `uv run` instead of the system interpreter, declaring dependencies inline so nothing is installed globally:
  - `uv run --with pyyaml python -c "..."`
  - `uv run --with requests --with rich script.py`
- The same pattern applies for any tool that has a `uv`-managed equivalent: declare dependencies inline at invocation time rather than relying on a pre-existing environment.
