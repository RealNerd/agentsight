# Contributing to AgentSight

Thanks for your interest in contributing! This guide covers everything you need to get started.

## Prerequisites

- **Rust stable toolchain** — install via [rustup](https://rustup.rs/)
- **cargo-deny** (optional, for dependency audits) — `cargo install cargo-deny`

## Building

```bash
cargo build
```

## Testing

```bash
cargo test
```

The project has three test layers:

1. **Unit tests** — inline `#[cfg(test)]` modules colocated with the code they test.
2. **API integration tests** (`tests/api_integration.rs`) — exercise the axum server layer using `tower::ServiceExt::oneshot()`, no TCP socket required.
3. **CLI integration tests** (`tests/cli_integration.rs`) — run the compiled `agentsight` binary via `assert_cmd` to verify argument parsing, dispatch, and end-to-end output.

New features and bug fixes should include tests. For bugs, write a failing test that reproduces the issue before applying the fix.

## Code Quality

CI runs all of the following on every push. PRs must pass.

```bash
cargo fmt --check          # Formatting
cargo clippy -- -D warnings # Lints (warnings are errors)
cargo deny check            # Dependency audit
cargo test                  # Tests
```

Run these locally before pushing to avoid CI failures.

## Architecture Overview

```
src/
├── main.rs              # CLI entry (clap derive)
├── commands/            # One file per subcommand
├── skills/              # Built-in CC skill definitions (embedded via include_str!)
├── parser/              # JSONL types, reader, session discovery
│   ├── types.rs         # Serde structs matching Claude Code JSONL schema
│   ├── reader.rs        # Parse files, summarize sessions
│   └── session_index.rs # Discover sessions from ~/.claude/projects/
├── cost/                # Pricing config → dollar amounts (optional layer)
├── config/              # Config loading, billing mode, pricing tables
└── output/              # Table (comfy-table) and JSON formatters
```

**Key design rule:** The parser and cost layers are pure — no side effects, no config awareness. The command layer is thin glue. The output layer is swappable (table vs JSON, decided at the command layer).

## PR Process

1. Fork the repo and create a feature branch.
2. Keep PRs small and focused — one logical change per PR.
3. Describe what changed and why in the PR description.
4. Include tests for new features and bug fixes.
5. Ensure CI passes (`cargo fmt`, `clippy`, `test`, `deny`).

## Code Style

- Run `cargo fmt` before committing — formatting is enforced in CI.
- Follow existing patterns in the codebase.
- No swallowed errors — empty `catch` blocks must have a comment explaining why.
- No `any` types without justification.
- Clippy warnings are treated as errors.

## Issue Reporting

Use [GitHub Issues](https://github.com/RealNerd/agentsight/issues). Please include:

- What you expected to happen
- What actually happened
- Your OS and Rust toolchain version (`rustc --version`)
- Steps to reproduce

## License

Contributions are accepted under the **MIT OR Apache-2.0** dual license, matching the project's `Cargo.toml`. By submitting a PR you agree to license your contribution under these terms.
