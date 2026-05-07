# Deep Review: AgentSight Full Project

> **Date**: 2026-05-07
> **Scope**: Full codebase — src/, static/, config/, tests/, Cargo.toml, documentation
> **Voices**: Security, Architect, Test, Integration, Docs, DX, Compliance
> **Focus**: Technical + documentation gaps (pre-Homebrew/open-source readiness)
> **Method**: Independent multi-voice deep-read with structured synthesis
> **Security mode**: Standard (single pass)
> **Assumption**: Nothing is correct until verified against source

---

## Voice 1: Security Engineer

### Verdict: SOLID FOR A LOCAL TOOL, BUT THE DASHBOARD HAS XSS SURFACE AND WIDE-OPEN CORS

**What's done right:**
- Server binds to `127.0.0.1` only — no network exposure (`server/mod.rs:80`)
- No secrets, API keys, or credentials anywhere in the codebase
- Session data is read-only — `~/.claude/` is never modified
- JSONL parsing gracefully handles malformed input (`parser/reader.rs:37-48`)
- File discovery validates UUID format before processing (`parser/session_index.rs:54-56`)
- Session ID lookups use in-memory prefix matching, not filesystem paths — no path traversal risk (`server/cache.rs:196-198`)

#### Dimension 1: Input Validation

**Gap 1 (LOW): No bounds on API query parameters**

API handlers accept `days` and `limit` without upper bounds. A request like `?days=999999` would scan and parse every session file on disk.

- `handlers.rs:39` — `limit` defaults to 50 but accepts any `u64`
- `handlers.rs:38` — `days` defaults to 7 but accepts any `u64`
- `handlers.rs:202` — summary `days` same issue

**Recommendation**: Add reasonable upper bounds (e.g., `days.min(365)`, `limit.min(500)`).

#### Dimension 2: Auth/AuthZ

No authentication on any endpoint. Acceptable for a localhost-only tool. No finding.

#### Dimension 3: Secrets Management

No secrets in source. No `.env` files. No credentials. No finding.

#### Dimension 4: Dependency/Supply Chain

All dependencies are well-known, high-download crates (axum, tokio, serde, clap, chrono). No suspicious packages. No `cargo audit` or `cargo deny` configured for ongoing monitoring — see Compliance voice.

#### Dimension 5: Permissions/Least Privilege

N/A — no IAM, no system permissions beyond filesystem reads.

#### Dimension 6: Runtime Exposure

**Gap 2 (MEDIUM): CORS allows any origin**

`server/mod.rs:22-25`:
```rust
let cors = CorsLayer::new()
    .allow_origin(Any)
    .allow_methods(Any)
    .allow_headers(Any);
```

Even though the server is localhost-only, any webpage in any browser tab can make requests to `http://127.0.0.1:3141/api/v1/*` and read session data (token counts, project paths, slugs, timestamps). A malicious page could silently exfiltrate usage patterns.

**Recommendation**: Restrict CORS to same-origin only, or use `allow_origin(["http://127.0.0.1:3141".parse().unwrap()])`.

**Gap 3 (LOW): Template-literal HTML injection in dashboard**

Session slugs and project names are interpolated directly into HTML via JavaScript template literals without escaping:

- `app.js:306`: `${s.slug || s.session_id.slice(0, 8)}`
- `app.js:800-810`: Gantt chart labels from session slugs
- `app.js:1000+`: Watch page session labels

Data source is local JSONL files the user generated, so exploitation requires write access to `~/.claude/`. Low risk but worth sanitizing for defense-in-depth.

**Recommendation**: Use `textContent` instead of `innerHTML` for user-derived strings, or add an `escapeHtml()` utility.

#### Recommended External Tools
- `cargo audit` — check for known vulnerabilities in dependencies
- `cargo deny` — license and advisory checking
- Trivy (`trivy fs .`) — comprehensive dependency + secret scanning

### Security Score: 7/10

---

## Voice 2: Software Architect

### Verdict: CLEAN LAYERED ARCHITECTURE WITH DUPLICATED AGGREGATION LOGIC BETWEEN CLI AND SERVER

**What's done right:**
- Parser layer is genuinely pure — no side effects, no config (`parser/types.rs`, `parser/reader.rs`)
- Cost layer is pure — deterministic token-to-dollar math (`cost/calculator.rs`)
- Two-layer cache uses file-size invalidation — correct for append-only JSONL (`server/cache.rs:36-37`)
- Output layer is cleanly swappable: table vs JSON decided at command level (`output/mod.rs`, `output/table.rs`, `output/json.rs`)
- `SessionEntry` enum with `#[serde(other)] Unknown` — forward-compatible with new JSONL types (`parser/types.rs:23-24`)
- Configuration merging: compiled-in defaults + user overrides with clean precedence (`config/mod.rs:43-66`)

**Gap 1 (HIGH): Duplicated aggregation logic between CLI and server**

The summary handler in `server/handlers.rs:169-348` reimplements aggregation logic that already exists in `commands/summary.rs`. Both build `by_hour`, `by_project`, `by_model` HashMaps from session data, but through separate codepaths. This has already created observable drift:

- CLI JSON output uses `"sessions"` key; API uses `"session_count"`
- CLI JSON output omits `by_project`, `by_day`, `cache_hit_ratio`, `avg_tokens_per_session`
- Server handler doesn't use `SummaryData` or `compute_summary()` from the CLI

This means bugs fixed in one path may not be fixed in the other.

**Recommendation**: Extract shared aggregation into a `compute_summary()` function in a shared module (not in `commands/`). Both CLI and server handlers should call it. The `SummaryJson` struct should be the single output shape.

**Gap 2 (MEDIUM): Hardcoded fallback pricing in 3 places**

The fallback `ModelPricing { input_per_million: 5.0, output_per_million: 25.0, ... }` is duplicated:
- `server/cache.rs:155-159`
- `server/watcher.rs:101-107`
- `commands/summary.rs` (implicit via config lookup)

If pricing changes, all three must be updated.

**Recommendation**: Add `impl Default for ModelPricing` with the Opus 4 pricing, or a `ModelPricing::opus_default()` const.

**Gap 3 (MEDIUM): diagnose.rs at 2,424 lines is overloaded**

`commands/diagnose.rs` contains cache classification, bash loop detection, context growth analysis, recommendation generation, CLAUDE.md analysis, per-project aggregation, and all associated types. It's the largest file by far.

**Recommendation**: Split into `diagnose/mod.rs`, `diagnose/cache.rs`, `diagnose/patterns.rs`, `diagnose/recommendations.rs`. The unit tests already suggest natural boundaries.

**Gap 4 (LOW): Session lookup by slug is O(n) scan**

`server/cache.rs:203-218` — `get_by_slug_best()` iterates all cached sessions for every slug lookup. With thousands of sessions, this could become noticeable.

**Recommendation**: Add a secondary index (HashMap<String, Vec<PathBuf>>) keyed by slug. Not urgent — current scale is fine.

### Architect Score: 7/10

---

## Voice 3: Test Engineer

### Verdict: STRONG UNIT AND FIXTURE COVERAGE, BUT ZERO SERVER/API TESTS AND NO CLI INTEGRATION TESTS

**What's done right:**
- 153 tests passing across 7 test suites
- 10 purpose-built JSONL fixtures covering edge cases: empty, malformed, large (99 turns), multi-model, bash-heavy, error-heavy, cache-churning, sidechain (`tests/fixtures/`)
- Fixture sanitization tool (`commands/sanitize.rs`) ensures no real user data in test fixtures
- Tests verify resilience: malformed JSON, empty sessions, zero tokens (`tests/parse_fixtures.rs`)
- `no_real_home_dirs_in_fixtures()` test catches accidental PII leaks in fixtures

**Gap 1 (HIGH): Zero test coverage for src/server/**

The entire server module — handlers, cache, watcher, SSE, routing — has no tests. This means:
- API response shapes are untested (JSON contract could break silently)
- Cache invalidation logic is untested
- SSE streaming is untested
- Route registration is untested

**Recommendation**: Add integration tests using `axum::test::TestServer` or `reqwest` against the router. At minimum: one test per API endpoint verifying response shape, one test for cache invalidation.

**Gap 2 (HIGH): No CLI binary integration tests**

No test runs `agentsight sessions --json` or similar and validates output. The main.rs dispatch logic is untested.

**Recommendation**: Add `tests/cli.rs` using `assert_cmd` crate to test CLI output with `--json` flag against fixture data via `--claude-dir`.

**Gap 3 (MEDIUM): No frontend tests**

The dashboard SPA (`app.js` at 52K, `charts.js`, `utils.js`) has no tests. Rendering logic, API client, chart creation — all untested.

**Recommendation**: For a project this size, at minimum add snapshot tests for the API client functions. Consider `vitest` or `playwright` if the dashboard becomes more critical.

**Gap 4 (LOW): No CI/CD pipeline**

No `.github/workflows/` directory. Tests, clippy, and fmt must be run manually.

**Recommendation**: Add a basic GitHub Actions workflow: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`. This is table-stakes for open source.

### Test Score: 5/10

---

## Voice 4: Integration/Contract Reviewer

### Verdict: CLI AND API PRODUCE DIFFERENT JSON SHAPES FOR THE SAME DATA

**What's done right:**
- `SummaryJson` struct in `output/json.rs` provides typed API responses
- `session_to_json()` shared helper ensures consistent session representation (`output/json.rs`)
- Frontend API client is clean and consistent (`utils.js:50-68`)

**Gap 1 (HIGH): CLI --json and API /summary return different schemas**

The CLI `summary --json` (in `commands/summary.rs:469-477`) builds an ad-hoc `serde_json::json!({})` with different field names and missing fields compared to `SummaryJson`:

| Field | CLI JSON | API JSON |
|-------|----------|----------|
| Session count | `"sessions"` | `"session_count"` |
| By project | missing | `"by_project"` |
| By day | missing | `"by_day"` |
| Cache hit ratio | missing | `"cache_hit_ratio"` |
| Avg tokens/session | missing | `"avg_tokens_per_session"` |
| Period days | missing | `"period_days"` |

Anyone scripting against `agentsight summary --json` will get a different shape than `curl localhost:3141/api/v1/summary`.

**Recommendation**: Use `SummaryJson` for both CLI and API output. The CLI should call `compute_summary()` → populate `SummaryJson` → serialize.

**Gap 2 (MEDIUM): Cost calculation differs between CLI and server**

CLI summary (summary.rs:266-271) calculates cost per-turn with model-specific pricing. Server handler (handlers.rs:208) uses session-level cost from the cache, which uses only the first model detected. Multi-model sessions will have different cost figures.

**Recommendation**: Ensure both paths use identical per-turn cost accumulation.

**Gap 3 (LOW): Frontend has no type safety against API**

`app.js` references `data.by_hour`, `data.session_count`, `data.by_project` etc. as raw property accesses. If a field is renamed or removed in `SummaryJson`, the frontend breaks with no error — charts just silently don't render.

**Recommendation**: Add a TypeScript types file or JSDoc annotations. Not urgent for current scale.

### Integration Score: 4/10

---

## Voice 5: Documentation QA

### Verdict: ZERO USER-FACING DOCUMENTATION — NO README, NO LICENSE, NO CHANGELOG

**What's done right:**
- `CLAUDE.md` is well-structured for developer orientation: architecture diagram, key commands, conventions
- `SPEC.md` is comprehensive: 17KB covering data flow, token model, boundaries, testing strategy
- `docs/actionable-decisions.md` provides strategic guidance on interpreting data
- `config/default_pricing.toml` has clear inline comments
- Auto-generated config template (`config/mod.rs:100-118`) explains each setting

**Gap 1 (HIGH): No README.md**

The repo is public on GitHub with no README. Visitors see a raw file listing. This is the single highest-impact documentation gap. A README should cover:
- What AgentSight is (one paragraph)
- Screenshot of dashboard and CLI output
- Installation (from source, eventually `brew install`)
- Quick start (3-5 commands)
- Feature overview
- Configuration

**Recommendation**: Create README.md before any distribution work (Homebrew, crates.io).

**Gap 2 (HIGH): No LICENSE file**

Public repo without a license = "all rights reserved." Nobody can legally fork, modify, or redistribute. This blocks Homebrew formula (taps require a license), crates.io publishing (requires `license` field), and community adoption.

**Recommendation**: Add LICENSE file. MIT or Apache-2.0 (or dual MIT/Apache-2.0 per Rust convention) are the standard choices.

**Gap 3 (MEDIUM): No CHANGELOG.md**

No version history. Users (and the Homebrew formula) need to know what changed between releases.

**Recommendation**: Start a CHANGELOG.md following Keep a Changelog format. Backfill is optional — start from the first tagged release.

**Gap 4 (MEDIUM): Cargo.toml missing publishing metadata**

`Cargo.toml` lacks: `authors`, `license`, `repository`, `homepage`, `readme`, `keywords`, `categories`. Required for `cargo publish`.

**Recommendation**: Add metadata fields before tagging v0.1.0.

**Gap 5 (LOW): No CONTRIBUTING.md**

No contributor guidelines. Lower priority — can be added when community contributions are expected.

### Docs Score: 2/10

---

## Voice 6: Developer Experience

### Verdict: EXCELLENT CLI ERGONOMICS WITH GAPS IN DISCOVERABILITY AND SHELL INTEGRATION

**What's done right:**
- `--json` flag on every command — excellent scriptability (`main.rs:17`)
- `--cost` flag / billing mode config — clean UX for different user types (`main.rs:20-21`, `config/mod.rs:69-73`)
- Auto-config creation on first run with commented template (`config/mod.rs:122-133`)
- Smart session matching: UUID prefix, slug substring, case-insensitive (`server/cache.rs:203-218`)
- Default port 3141 avoids common conflicts (`main.rs:137`)
- Graceful shutdown with ctrl-c handler (`server/mod.rs:88-93`)
- `shorten_project()` and `shorten_model()` keep table output readable (`output/table.rs:361-398`)

**Gap 1 (MEDIUM): No shell completions**

No shell completion generation for bash, zsh, or fish. Clap supports this natively via `clap_complete`.

**Recommendation**: Add a `completions` subcommand or generate at build time.

**Gap 2 (MEDIUM): Minimal --help examples**

Clap `about` strings describe what each command does but don't show usage examples. Users have to guess syntax.

**Recommendation**: Add `#[command(after_help = "Examples:\n  agentsight session my-feature\n  ...")]` to key subcommands.

**Gap 3 (LOW): No `--version` output beyond version number**

`agentsight --version` shows `agentsight 0.1.0` but no build info (git hash, build date). Useful for bug reports.

**Recommendation**: Add build-time info via `built` or `vergen` crate.

### DX Score: 7/10

---

## Voice 7: Compliance/Privacy/Legal

### Verdict: PUBLIC REPO WITH NO LICENSE IS A LEGAL BLOCKER FOR DISTRIBUTION

**What's done right:**
- No PII collection or transmission — tool reads local files only
- No telemetry (planned as opt-in per `agentsight-chc`)
- No network requests from the CLI (except localhost dashboard)
- Session sanitization tool strips real paths and content (`sanitize/mod.rs`, `sanitize/content.rs`)

#### Regulatory Compliance
N/A — no user data collection, no network services, no accounts.

#### OSS License Risk

**Gap 1 (HIGH): No LICENSE file in a public repository**

The repo at `github.com/RealNerd/agentsight` is public with no license. Under copyright law, this means all rights are reserved by default. Consequences:
- Nobody can fork or redistribute
- Homebrew taps require a license to reference
- `cargo publish` requires a `license` or `license-file` field
- Potential users in corporate environments cannot adopt it

**Recommendation**: Add a LICENSE file immediately. Dual MIT/Apache-2.0 is Rust convention. Add `license = "MIT OR Apache-2.0"` to Cargo.toml.

**Gap 2 (MEDIUM): No dependency license audit**

25 direct dependencies and many transitive ones. All appear to be MIT/Apache-2.0 based on crate conventions, but no automated audit has verified this. A single AGPL or GPL transitive dependency would create viral licensing obligations.

**Recommendation**: Run `cargo deny check licenses` or `trivy fs --scanners license .` and add to CI.

**Gap 3 (LOW): No attribution file**

MIT and Apache-2.0 licenses require copyright notices to be preserved. When distributing a compiled binary (e.g., via Homebrew), a NOTICES or THIRD-PARTY file listing dependency licenses is best practice.

**Recommendation**: Generate via `cargo about` or `cargo license`.

### Compliance Score: 3/10

---

## Synthesis: Prioritized Gap List

### P0 -- Ship Blockers
| # | Finding | Severity | Voices | Fix |
|---|---------|----------|--------|-----|
| 1 | No LICENSE file — blocks Homebrew, crates.io, legal use | HIGH | Docs, Compliance | Add `LICENSE-MIT` + `LICENSE-APACHE` files, add `license` to Cargo.toml |
| 2 | No README.md — public repo with no user-facing docs | HIGH | Docs | Create README with install, quick start, screenshots |

### P1 -- Fix Before GA
| # | Finding | Severity | Voices | Fix |
|---|---------|----------|--------|-----|
| 1 | CLI and API return different JSON schemas for summary | HIGH | Integration, Architect | Use `SummaryJson` for both CLI and API output |
| 2 | Zero server/API test coverage | HIGH | Test | Add `axum::test` integration tests for each endpoint |
| 3 | Duplicated aggregation logic (CLI vs server) | HIGH | Architect | Extract shared `compute_summary()` into a shared module |
| 4 | No CI/CD pipeline | MEDIUM | Test | Add GitHub Actions: fmt, clippy, test, build |
| 5 | CORS allows any origin on dashboard | MEDIUM | Security | Restrict to same-origin or localhost only |
| 6 | Cargo.toml missing publishing metadata | MEDIUM | Docs | Add authors, license, repository, homepage, keywords |
| 7 | No CHANGELOG.md | MEDIUM | Docs | Start changelog before first tagged release |
| 8 | No dependency license audit | MEDIUM | Compliance | Run `cargo deny` and add to CI |

### P2 -- Nice to Have
| # | Finding | Severity | Voices | Fix |
|---|---------|----------|--------|-----|
| 1 | diagnose.rs at 2,424 lines | MEDIUM | Architect | Split into submodules |
| 2 | Hardcoded fallback pricing in 3 places | MEDIUM | Architect | Add `ModelPricing::default()` |
| 3 | No shell completions | MEDIUM | DX | Add via `clap_complete` |
| 4 | No CLI binary integration tests | MEDIUM | Test | Add `assert_cmd` tests |
| 5 | No --help examples | LOW | DX | Add `after_help` to clap subcommands |
| 6 | XSS via template literals in dashboard | LOW | Security | Use `textContent` or add `escapeHtml()` |
| 7 | No bounds on API query params (days, limit) | LOW | Security | Clamp to reasonable maximums |
| 8 | No --version build info (git hash) | LOW | DX | Add `built` or `vergen` crate |
| 9 | No CONTRIBUTING.md | LOW | Docs | Add when community contributions expected |

---

## Metadata
- **Files reviewed**: 38 source files + 10 fixtures + 4 static assets + configs
- **Voices active**: 7/10
- **Scope focus**: Technical + documentation (pre-distribution readiness)
- **Generated by**: `/deep-review` multi-voice analysis
