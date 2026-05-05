# AgentSight

## Vision

AgentSight is a **session intelligence and observability platform for agentic coding**. It gives developers visibility into what their AI coding agents (Claude Code, Cursor, Copilot, Codex, etc.) are actually doing, where tokens go, and what makes sessions succeed or fail.

**Tagline:** "Datadog for agentic coding sessions."

**Target users:** Developers and teams using AI coding agents who need cost visibility, session debugging, and context optimization.

## Problem Statement

When an agentic coding session burns tokens and produces poor results, developers have zero forensics. They cannot answer:

- Where did my tokens actually go? (file reads, retries, dead-end explorations, thinking)
- What context was the agent working with when it made a bad decision?
- Which of my CLAUDE.md / .cursorrules instructions are helping vs. actively hurting?
- Why did this session cost $8 when yesterday's identical task cost $1.50?
- What patterns across my sessions consistently waste tokens?

Existing observability tools (Helicone, LangSmith, Arize, Datadog LLM) target **production LLM inference** — monitoring API endpoints for apps. Nobody is building for the **developer-as-user-of-agents** use case.

## Core Product Pillars

### 1. Token Attribution
Break down every session into: context loading, file reads, tool calls, retries, thinking, output. Show where waste happens at the task/file/decision level. Answer "where did my money go?" for every session.

### 2. Context Effectiveness Scoring
Measure the impact of CLAUDE.md / .cursorrules / context file instructions on actual session outcomes. A/B test instructions. Surface which instructions save tokens vs. cause retry loops. ETH Zurich research showed AGENTS.md files consistently increase steps to completion — developers need data, not cargo-culting.

### 3. Session Replay & Debugging
When a session goes sideways, scrub through the agent's decision tree. See what it read, what it decided, where it diverged. Chrome DevTools network tab, but for agent reasoning. Identify the exact moment a session went off the rails.

### 4. Cross-Session Pattern Detection
Aggregate intelligence across sessions: "Your agents spend 30% of tokens re-reading the same 5 files — here's a context snippet to eliminate that." Or: "Sessions touching file X average 3x cost — here's why." Learn from a developer's history to optimize future sessions.

### 5. Cost Forecasting
Before hitting enter on a prompt, estimate the likely token cost based on historical patterns for similar tasks in the developer's codebase. Help developers make informed decisions about when to use agents vs. do it manually.

## Market Timing & Tailwinds

- **GitHub Copilot moved to usage-based billing** (June 2026) — cost visibility now matters to every developer, not just API power users
- **The 2026 developer question:** "Which tool won't torch my credits?" — AgentSight answers this regardless of which agent they use
- **Context engineering is exploding** as a discipline but has zero measurement tooling
- **Agent solutions pass benchmarks but merge at half the rate of human solutions** — the gap between benchmark and production is where observability matters

## Competitive Landscape

| Player | Focus | Gap |
|--------|-------|-----|
| Helicone | Production LLM API monitoring | Not developer-session-oriented |
| LangSmith | LangChain app tracing | Tied to LangChain ecosystem |
| Arize/Phoenix | ML observability | Production inference, not dev sessions |
| Datadog LLM | Enterprise APM add-on | Overkill for individual developers |
| Braintrust | Eval + logging | Focused on app builders, not agent users |
| **AgentSight** | **Developer agent session intelligence** | **Unclaimed** |

## Strategic Moat

The proprietary dataset of "what makes agentic coding sessions succeed or fail" across codebases, agent tools, and developer patterns. No one else is collecting this. This becomes the foundation for:
- Predictive cost models
- Auto-generated optimal context files
- Agent selection recommendations (when to use Opus vs Sonnet vs Haiku)
- Benchmarking that reflects real-world developer workflows, not synthetic evals

## Key Design Principles

- **Agent-agnostic**: Must work across Claude Code, Cursor, Copilot, Codex, and future tools
- **Zero-friction capture**: Hook-based, CLI-based, or extension-based — no manual logging
- **Privacy-first**: Code content stays local; only metadata/telemetry flows to the platform
- **Developer-first UX**: Not an enterprise dashboard — a tool developers actually want open

## Technical Considerations

### Data Capture Strategies (to investigate)
- Claude Code hooks system (pre/post tool call events)
- CLI wrapper/proxy approach (similar to RTK architecture)
- VS Code / Cursor extension that intercepts agent communication
- MCP server that acts as an observability sidecar
- Log file parsing (Claude Code, Cursor, Copilot all produce session logs)

### Potential Architecture
- Local agent/daemon for capture (privacy: code stays on machine)
- Lightweight metadata extraction and anonymization
- Cloud dashboard for visualization, cross-session analysis, and pattern detection
- API for integrations and custom reporting

## Open Questions (Scoping Phase)

- [ ] Which capture method has the best coverage-to-effort ratio? (hooks vs proxy vs extension vs log parsing)
- [ ] What's the MVP — just cost attribution? Or does session replay need to ship in v1?
- [ ] Freemium vs paid-from-day-one? (cost attribution free, intelligence features paid?)
- [ ] Solo developer focus first, or team/org features from the start?
- [ ] What data can be captured without violating ToS of each agent tool?
- [ ] Build as CLI tool first (matches developer workflow) or web dashboard first?
- [ ] What's the right level of code content to capture vs. pure metadata?
- [ ] Can we partner with agent tool vendors (Anthropic, Cursor) for first-party integration?

## Development Commands

```bash
# TBD — project not yet initialized
```

## References & Research

- [The Agentic Coding Stack: Missing Link Nobody Has Built](https://blog.devgenius.io/the-agentic-coding-stack-7-tools-5-layers-and-the-missing-link-nobody-has-built-yet-de264b260db3)
- [Anthropic 2026 Agentic Coding Trends Report](https://resources.anthropic.com/2026-agentic-coding-trends-report)
- [ETH Zurich: AGENTS.md Files Increase Task Steps](https://www.infoq.com/news/2026/03/agents-context-file-value-review/)
- [The Real Cost of AI Coding in 2026](https://www.morphllm.com/ai-coding-costs)
- [AI Cost Observability: Measuring Token Spend](https://www.vantage.sh/blog/finops-for-ai-token-costs)
- [Agentic AI Coding Costs: "Which Tool Won't Torch My Credits?"](https://byteiota.com/agentic-coding-economics/)
- [GitHub Copilot Usage-Based Billing](https://github.blog/news-insights/company-news/github-copilot-is-moving-to-usage-based-billing/)
- [Runtime Observability for AI Coding Agents (HN)](https://news.ycombinator.com/item?id=47281152)
- [Martin Fowler: Context Engineering for Coding Agents](https://martinfowler.com/articles/exploring-gen-ai/context-engineering-coding-agents.html)
- [7 Best AI Agent Observability Tools for Coding Teams](https://www.augmentcode.com/tools/best-ai-agent-observability-tools)
