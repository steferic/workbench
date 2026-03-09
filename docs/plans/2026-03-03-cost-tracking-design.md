# Cost/Token Tracking Design

## Summary

Track token usage and estimated costs for Claude and Codex sessions by reading their JSONL log files. Display a global cost total in the status bar. Update on session idle.

## Supported Agents

| Agent | Log Location | Format |
|-------|-------------|--------|
| Claude | `~/.claude/projects/<hash>/*.jsonl` | Per-turn `message.usage` with input/output/cache tokens |
| Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` | `token_count` events with cumulative totals |
| Gemini | N/A | No structured logs available |
| Grok | N/A | No standardized CLI |

## Data Flow

1. Session goes idle (2s no output — already detected via `newly_idle` in `update_idle_queue()`)
2. For Claude/Codex sessions: spawn background thread to find and parse latest JSONL log
3. Thread sends `Action::CostUpdated(session_id, SessionCost)` back via the action channel
4. State stores cost in `HashMap<Uuid, SessionCost>`
5. Status bar renders global total

## Log File Matching

### Claude
- Project hash derived from workspace `working_dir` path
- Location: `~/.claude/projects/<hash>/`
- Find most recent `.jsonl` file modified after session's `started_at`
- Parse each line for `message.usage` fields, sum all turns

### Codex
- Location: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`
- Date from session's `started_at`
- Find files modified after session start
- Parse `token_count` events, take last (cumulative) value

## Data Model

```rust
#[derive(Debug, Clone, Default)]
pub struct SessionCost {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub estimated_cost_usd: f64,
}
```

Stored in: `HashMap<Uuid, SessionCost>` on state (system or data — TBD during implementation).

## Cost Calculation

Hardcoded pricing per model (easy to update):

| Model | Input (per 1M) | Output (per 1M) | Cache Read | Cache Write |
|-------|----------------|-----------------|------------|-------------|
| Claude Sonnet | $3.00 | $15.00 | $0.30 | $3.75 |
| Claude Opus | $15.00 | $75.00 | $1.50 | $18.75 |
| Codex (GPT-4.1) | $2.00 | $8.00 | $0.50 | $2.00 |

These are approximations. Exact model detection from logs when possible.

## Display

Status bar (bottom): `$1.23` right-aligned

Color coding:
- Green: < $1
- Yellow: $1 - $10
- Red: > $10

## Update Trigger

Only when a session transitions to idle (joins `newly_idle` list). No polling. Minimal disk I/O.
