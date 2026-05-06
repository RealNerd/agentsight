# Actionable Decisions from AgentSight Data

## Cache Hit Ratio — The Single Most Important Number

Your average is 93.5%, which is strong. But the variance matters more than the average.

- **Sessions below 80%**: Context is churning. The agent is re-encoding large amounts of context every turn instead of reusing it. This happens when sessions jump between unrelated tasks, when CLAUDE.md is very large and changes often, or when the agent is reading lots of different files. **Action:** Break multi-topic sessions into focused ones. One task per session.

- **Sessions above 95%**: The agent is efficiently reusing context. These are your model sessions — look at what they have in common (project, task type, session length) and replicate the pattern.

- **Sessions with anomalously low cache hit**: Almost half the tokens were cold context. Worth investigating — was it a brand new project? A session that jumped around? That's the kind of session that burns through your Max usage budget fastest.

## Token Breakdown — Where the Weight Is

The four buckets tell you different things:

| If this is high... | It means... | Action |
|---|---|---|
| **Cache creation** | Agent is encoding new context every turn | Session is doing too many different things, or reading too many files the agent hasn't seen before |
| **Cache read** | Context is being reused (good) | This is the cheapest bucket — high is desirable |
| **Input** | Base prompt is large | Your CLAUDE.md or system instructions might be bloated |
| **Output** | Agent is generating a lot | Could be verbose responses, large code blocks, or extended thinking |

On Max, the practical concern isn't cost — it's **throughput**. Every token counts against your usage limits. A session that's 97% cache reads is using your allocation ~10x more efficiently than one that's re-encoding everything from scratch.

## Tool Calls — Spotting Waste Patterns

Look at the tool breakdown for patterns:

- **High Read count relative to Edit/Write**: The agent is reading a lot but producing little. It may be exploring without direction. **Action:** Give more specific instructions upfront, or point it to the right files in your prompt.

- **Repeated Read on the same files across sessions**: If the agent is re-reading the same core files every session, you could add a context summary to your project's CLAUDE.md that eliminates those reads.

- **High Bash count**: Trial-and-error debugging. The agent is running commands to figure out what's going on instead of reasoning from context. **Action:** Better error messages in your CLAUDE.md, or provide the agent with known-good commands.

- **Task/subagent calls**: These spawn child sessions with their own token overhead. A session with many Task calls is multiplying context loading. Sometimes that's necessary, sometimes a direct approach would use fewer tokens.

## Cross-Session Patterns — The Strategic View

- **Identify your top token-consuming projects.** These are where optimization effort pays off most. Even a 10% efficiency improvement on your top two projects saves more than perfecting everything else combined.

- **Track daily token burn.** On Max, you have a daily and monthly token budget. If you're hitting limits, the data tells you which project or session pattern pushed you over.

## Concrete Decisions This Enables

1. **"Should I start a new session or continue this one?"** — Check your current session's cache hit ratio with `watch`. If it's dropping below 85%, a fresh session might be more efficient.

2. **"Which project needs a better CLAUDE.md?"** — The project with the highest tokens-per-session average is the one where the agent is working hardest to understand context. Invest in instructions there first.

3. **"Am I going to hit my limit today?"** — `summary --days 1` shows your daily burn rate. Compare against your Max tier's daily budget.

4. **"What went wrong in that session?"** — `session <slug>` shows you the turn-by-turn. A spike in cache creation mid-session usually means the agent lost context (compaction happened) or pivoted to a new task.

5. **"Is my CLAUDE.md helping or hurting?"** — Compare cache creation tokens across sessions in the same project. If sessions after you updated CLAUDE.md show higher cache creation, the new instructions might be too large or causing the agent to explore more.

## The Bash Loop Problem

One of the most expensive anti-patterns is invisible to most developers: the agent gets stuck retrying a failing command. It runs `cargo test`, gets an error, tries again, gets the same error, tries a slight variation, same error — burning through tokens doing nothing useful. This shows up in session data as consecutive turns where the only tool call is Bash.

**Detection:** 3+ consecutive Bash-only turns in the turn-by-turn data. Even without seeing the command text, this pattern almost always means the agent is stuck.

**What to put in CLAUDE.md to prevent it:**
- Known-good build, test, and lint commands with expected output
- Common error messages and their fixes ("If cargo test fails with X, check Y")
- A rule: "If a shell command fails twice with the same error, stop and ask the user rather than retrying"

## CLAUDE.md Hygiene

Most developers either write too much CLAUDE.md (bloating every turn's input) or write nothing (forcing the agent to explore every session). Both waste tokens.

**Signs your CLAUDE.md is too large:**
- High base input tokens even on turn 1 (before the agent has read any files)
- Consistent cache creation on early turns across multiple sessions

**Signs your CLAUDE.md is missing or insufficient:**
- High Read/Glob tool calls in the first 5 turns of every session (agent exploring to understand the project)
- Low cache hit ratio in early turns that improves later (agent building context from scratch each time)
- Same files being Read in multiple sessions (the agent keeps rediscovering what it should already know)

**What belongs in CLAUDE.md:**
- Build/test/lint commands (the agent will try to figure these out via Bash if you don't tell it)
- Key source file locations and their purpose (prevents exploration-heavy sessions)
- Project-specific conventions the agent can't infer from code alone
- Error recovery guidance for common failure modes

**What doesn't belong in CLAUDE.md:**
- Full API references (too large — put in a separate file and @-reference only when relevant)
- Obvious things the agent can infer from the codebase
- Instructions that change frequently (causes cache churn)

## The Feedback Loop — Where AgentSight Is Headed

The current tool answers "where did tokens go?" The next step is closing the loop:

1. **`agentsight diagnose`** — Analyzes sessions and produces specific, actionable recommendations (not just metrics)
2. **`/agentsight-diagnose` CC skill** — A Claude Code skill that runs diagnose, reads your CLAUDE.md, and tells you exactly what to change
3. **`agentsight init`** — First-run baseline report that gives new users a narrative starting point

The goal: install agentsight, run the skill, get personalized guidance on improving your CC workflow — without needing to understand token economics yourself. The dashboard and CLI remain available for power users who want the full picture.
