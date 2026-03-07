# RFC: Active Memory System for Claude Code

## Replacing the 200-line MEMORY.md with Always-On Consolidation

### Problem Statement

Claude Code's auto-memory is a flat file — `~/.claude/projects/<project>/memory/MEMORY.md` — with a hard 200-line cap. Only the first 200 lines are injected into the system prompt at session start. Topic files (`debugging.md`, `patterns.md`, etc.) exist on disk but aren't loaded automatically; Claude must explicitly read them with file tools mid-session, which it often doesn't.

This creates three compounding failures:

1. **Capacity ceiling.** 200 lines is roughly 1,500 tokens. A complex project generates far more worth-remembering knowledge than this across dozens of sessions.
2. **No active processing.** Memories are written as raw notes during a session and never revisited. There's no deduplication, no cross-referencing, no compression. Stale entries crowd out useful ones.
3. **No consolidation.** A human brain doesn't just store memories — it replays them during sleep, finds connections, and compresses them. Claude Code's memory never does this. Session 14's insight about the caching layer is never connected to session 3's architecture decision.

The always-on-memory-agent concept (Google ADK + Gemini Flash-Lite) solves this with a background daemon that continuously ingests, consolidates, and serves structured memory. This RFC proposes building equivalent functionality directly into Claude Code's core as a Rust subsystem, using Haiku via the Anthropic API for cheap background processing.

---

## Design Principles

**Invisible.** The user never configures, invokes, or thinks about the memory system. It replaces the current MEMORY.md mechanism transparently. No slash commands, no MCP servers, no plugins.

**Token-neutral.** The system must inject the *same or fewer* tokens into the system prompt as the current 200-line MEMORY.md, but with dramatically better information density. It achieves this by using Haiku to compress and prioritize memories before injection, rather than dumping raw notes.

**Always-on.** A lightweight background process runs between sessions, consolidating and compressing memories. It's not triggered by user action — it runs on a timer, like the human brain consolidating during sleep.

**Backward-compatible.** Existing CLAUDE.md files, `.claude/rules/`, and the memory hierarchy are untouched. This replaces only the auto-memory subsystem (MEMORY.md + topic files).

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│                    Claude Code Session                    │
│                                                          │
│  System prompt at session start:                         │
│  ┌──────────────┐  ┌──────────────┐  ┌───────────────┐  │
│  │  CLAUDE.md    │  │ .claude/     │  │ Active Memory │  │
│  │  (unchanged)  │  │ rules/       │  │ Summary       │  │
│  │              │  │ (unchanged)  │  │ (~1500 tokens) │  │
│  └──────────────┘  └──────────────┘  └───────┬───────┘  │
│                                              │          │
│  During session:                             │          │
│  ┌──────────────────────────────────┐        │          │
│  │ Memory Write Tool (existing)     │        │          │
│  │ Claude writes observations as    │        │          │
│  │ it works, same as today          ├───┐    │          │
│  └──────────────────────────────────┘   │    │          │
│                                          │    │          │
└──────────────────────────────────────────┼────┼──────────┘
                                           │    │
                    ┌──────────────────────┼────┼──────────┐
                    │   Memory Daemon      │    │   (Rust)  │
                    │   (claude-memoryd)    │    │          │
                    │                      ▼    │          │
                    │  ┌─────────────────────┐  │          │
                    │  │   Ingest Pipeline    │  │          │
                    │  │   (on write event)   │  │          │
                    │  │                     │  │          │
                    │  │  Raw note → Haiku → │  │          │
                    │  │  structured memory  │  │          │
                    │  └─────────┬───────────┘  │          │
                    │            │              │          │
                    │            ▼              │          │
                    │  ┌─────────────────────┐  │          │
                    │  │   SQLite Store       │  │          │
                    │  │   memory.db          │  │          │
                    │  │                     │  │          │
                    │  │  memories table     │  │          │
                    │  │  consolidations     │  │          │
                    │  │  session_index      │  │          │
                    │  └─────────┬───────────┘  │          │
                    │            │              │          │
                    │            ▼              ▲          │
                    │  ┌─────────────────────┐  │          │
                    │  │  Consolidation Loop  │  │          │
                    │  │  (every N minutes)   │  │          │
                    │  │                     │  │          │
                    │  │  Dedup, connect,    │  │          │
                    │  │  compress, insight  │──┘          │
                    │  └─────────────────────┘  (query     │
                    │                          at session  │
                    │                          start)     │
                    └──────────────────────────────────────┘
```

---

## Component Design

### 1. The Rust Daemon: `claude-memoryd`

A single-binary background process, lifecycle-managed by Claude Code. When Claude Code starts a session for a project, it ensures the daemon is running for that project. The daemon persists between sessions (with idle timeout) so consolidation happens even when you're not coding.

**Process lifecycle:**

```
claude code session start
  → check if memoryd running for this project (pidfile)
  → if not, spawn: claude-memoryd --project <path> --db <path>/memory.db
      (inherits ANTHROPIC_API_KEY + auth headers from parent env)
  → query daemon for session context
  → inject into system prompt

claude code session end
  → send final session summary to daemon
  → daemon continues running (idle timeout: 2 hours)

daemon idle timeout
  → daemon exits, writes clean state
  → next session respawns it
```

**Crate dependencies:**

```toml
[dependencies]
rusqlite = { version = "0.31", features = ["bundled"] }  # SQLite, no system dep
tokio = { version = "1", features = ["full"] }           # Async runtime
reqwest = { version = "0.12", features = ["json"] }      # Anthropic API calls
serde = { version = "1", features = ["derive"] }
serde_json = "1"
notify = "6"                                              # File watcher
tracing = "0.1"                                           # Structured logging
```

**IPC:** Unix domain socket at `~/.claude/projects/<project>/memoryd.sock`. The Claude Code TypeScript process communicates via simple JSON-over-socket protocol. No HTTP overhead, no port allocation.

```
// Request: get session context
{"method": "get_context", "params": {"max_tokens": 1500, "session_id": "abc"}}

// Response
{"context": "# Project Memory\n\n## Architecture...", "token_estimate": 1340}

// Request: daemon asks session to refresh its auth (sent daemon → session)
{"method": "auth_refresh_needed", "params": {}}

// Response: session sends back fresh env vars
{"env": {"ANTHROPIC_API_KEY": "sk-ant-...", "ANTHROPIC_BASE_URL": "https://..."}}
```

**Auth: inherited from the parent Claude Code process.**

The daemon doesn't manage its own credentials. When Claude Code spawns `claude-memoryd`, it passes the user's existing auth context through the process environment — the same env vars that Claude Code itself uses to authenticate with the Anthropic API.

```rust
// In the daemon's Haiku client (src/api/haiku.rs):

pub struct HaikuClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl HaikuClient {
    pub fn from_env() -> Result<Self, AuthError> {
        // Inherit auth exactly as Claude Code does.
        // The daemon is a child process — these are already in the env.
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .or_else(|_| std::env::var("CLAUDE_CODE_API_KEY"))
            .map_err(|_| AuthError::NoCredentials)?;

        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .unwrap_or_else(|_| "https://api.anthropic.com".to_string());

        Ok(Self {
            http: reqwest::Client::new(),
            api_key,
            base_url,
        })
    }

    pub async fn complete(&self, system: &str, user_msg: &str) -> Result<String, ApiError> {
        let resp = self.http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&serde_json::json!({
                "model": "claude-haiku-4-5-20251001",
                "max_tokens": 1024,
                "system": system,
                "messages": [{"role": "user", "content": user_msg}]
            }))
            .send()
            .await?;

        // Parse response, extract text content block
        let body: ApiResponse = resp.json().await?;
        Ok(body.content.iter()
            .filter_map(|b| if b.r#type == "text" { Some(b.text.as_str()) } else { None })
            .collect::<Vec<_>>()
            .join(""))
    }
}
```

On the TypeScript side, the spawn ensures the env is forwarded:

```typescript
// In Claude Code's daemon manager:

async function spawnMemoryDaemon(projectPath: string): Promise<ChildProcess> {
    const dbPath = getMemoryDbPath(projectPath);
    const sockPath = getMemorySockPath(projectPath);

    const child = spawn(getMemorydBinary(), [
        '--project', projectPath,
        '--db', dbPath,
        '--socket', sockPath,
    ], {
        // Key: inherit the full environment. The daemon gets
        // ANTHROPIC_API_KEY, ANTHROPIC_BASE_URL, and any other
        // auth context (OAuth tokens, proxy settings, etc.)
        // without us explicitly plumbing each one.
        env: process.env,
        stdio: 'ignore',
        detached: true,  // survives parent exit for idle consolidation
    });

    child.unref();  // don't keep Claude Code alive waiting on daemon

    // Write pidfile for reconnection by future sessions
    await fs.writeFile(
        path.join(getMemoryDir(projectPath), 'memoryd.pid'),
        String(child.pid)
    );

    // Wait for socket to appear (daemon is ready)
    await waitForSocket(sockPath, { timeout: 5000 });

    return child;
}
```

**Why this works cleanly:**

- **No new config.** The user already authenticated with Claude Code. The daemon piggybacks on that. Zero additional setup.
- **OAuth / SSO flows just work.** If the user authenticates via `claude login` (which sets up OAuth tokens in the environment), the daemon inherits those tokens. No separate auth flow for the background process.
- **API key users just work.** If `ANTHROPIC_API_KEY` is set, the daemon sees it. If the user uses a custom `ANTHROPIC_BASE_URL` (enterprise proxy, etc.), the daemon respects it.
- **Token refresh.** For OAuth-based auth, if the daemon's token expires during idle consolidation, it catches the 401 and queues work for retry. The next session spawn refreshes the env. The daemon can also request a fresh token via IPC from the next connecting session.
- **Billing.** Haiku calls made by the daemon bill to the same account as the user's Claude Code usage. No surprise — the user is already paying for Claude Code, and Haiku consolidation costs ~$0.02/day. This appears as normal API usage on their bill.

### 2. The SQLite Schema

Located at `~/.claude/projects/<project>/memory.db`, replacing the `memory/` directory of markdown files.

```sql
-- Individual memory atoms. One per observation.
CREATE TABLE memories (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    content       TEXT NOT NULL,           -- original raw note from Claude
    summary       TEXT,                    -- Haiku-compressed single line
    entities      TEXT,                    -- JSON: ["AuthService", "Redis", "JWT"]
    topics        TEXT,                    -- JSON: ["auth", "caching", "security"]
    memory_type   TEXT NOT NULL,           -- 'architecture', 'decision', 'pattern',
                                           -- 'gotcha', 'preference', 'progress'
    importance    REAL DEFAULT 0.5,        -- 0.0-1.0, set by Haiku
    source_session TEXT,                   -- session ID that produced this
    consolidated  INTEGER DEFAULT 0,       -- has this been processed by consolidator?
    decay_at      TEXT,                    -- NULL = permanent, else ISO8601 expiry
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Cross-cutting insights from consolidation passes.
CREATE TABLE consolidations (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    memory_ids    TEXT NOT NULL,           -- JSON array of connected memory IDs
    insight       TEXT NOT NULL,           -- the synthesized connection
    topics        TEXT,                    -- JSON: merged topic set
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Lightweight full-text search (no embeddings needed).
CREATE VIRTUAL TABLE memory_fts USING fts5(
    summary, content, entities, topics,
    content='memories',
    content_rowid='id'
);

-- Track what was injected per session to avoid repetition.
CREATE TABLE session_context (
    session_id    TEXT NOT NULL,
    memory_ids    TEXT NOT NULL,           -- JSON: which memories were shown
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_memories_type ON memories(memory_type);
CREATE INDEX idx_memories_importance ON memories(importance DESC);
CREATE INDEX idx_memories_consolidated ON memories(consolidated);
CREATE INDEX idx_memories_decay ON memories(decay_at);
```

### 3. Ingest Pipeline

When Claude writes to memory during a session (the same "Writing memory" action it does today), instead of appending to MEMORY.md, the write goes to the daemon.

**Current flow (replaced):**
```
Claude decides to save a note
  → writes text to ~/.claude/projects/<project>/memory/MEMORY.md
  → raw markdown, no processing
```

**New flow:**
```
Claude decides to save a note
  → sends raw note to memoryd via IPC
  → memoryd calls Haiku (async, non-blocking to session):

    System: You are a memory processor. Extract structured information.
    Return JSON only, no markdown.

    User: Process this observation from a coding session on project
    "{project_description}":

    "{raw_note}"

    Return:
    {
      "summary": "one line, max 20 words",
      "entities": ["list", "of", "proper nouns and key terms"],
      "topics": ["category", "tags"],
      "memory_type": "architecture|decision|pattern|gotcha|preference|progress",
      "importance": 0.0-1.0,
      "is_duplicate_of": null or "summary of existing memory it duplicates"
    }

  → memoryd stores in SQLite (or merges if duplicate detected)
  → memoryd updates FTS index
```

**Deduplication:** Before storing, the daemon queries FTS for similar existing memories. If Haiku's response includes `is_duplicate_of`, the daemon does a Jaccard similarity check on tokenized summaries. Above 0.6 overlap → supersede the old entry rather than creating a new one. This prevents the "same fact recorded 5 times across sessions" problem.

**Decay:** Memories of type `progress` get `decay_at` set to 7 days out. Type `preference` and `gotcha` get 90 days. Types `architecture`, `decision`, and `pattern` are permanent (NULL decay). A daily cleanup pass deletes expired entries.

### 4. Consolidation Loop

This is the core differentiator — the "brain during sleep" mechanism. Runs on a `tokio::time::interval`, default every 30 minutes when the daemon is alive.

```rust
async fn consolidation_tick(db: &Connection, api: &AnthropicClient) {
    // 1. Gather unconsolidated memories
    let memories = db.query(
        "SELECT * FROM memories WHERE consolidated = 0 ORDER BY created_at DESC LIMIT 50"
    );

    if memories.len() < 3 { return; } // not enough to consolidate

    // 2. Also pull recent consolidations for context
    let recent_insights = db.query(
        "SELECT * FROM consolidations ORDER BY created_at DESC LIMIT 10"
    );

    // 3. Ask Haiku to find connections
    let response = api.message(Message {
        model: "claude-haiku-4-5-20251001",
        max_tokens: 1000,
        system: "You are a memory consolidation system. You find connections
                 between memories, generate cross-cutting insights, and identify
                 memories that can be merged or compressed.
                 Return JSON only.",
        messages: [UserMessage {
            content: format!(
                "Here are recent unconsolidated memories:\n{}\n\n\
                 Here are existing insights:\n{}\n\n\
                 Find:\n\
                 1. connections: pairs of memory IDs that relate to each other\n\
                 2. insights: cross-cutting observations (max 3)\n\
                 3. merge_candidates: memories saying the same thing differently\n\
                 4. obsolete: memory IDs superseded by newer information\n\n\
                 Return: {{\"connections\": [...], \"insights\": [...],\
                          \"merge_candidates\": [...], \"obsolete\": [...]}}",
                format_memories(&memories),
                format_insights(&recent_insights)
            )
        }]
    });

    // 4. Apply results
    //    - Store new consolidation records
    //    - Mark processed memories as consolidated = 1
    //    - Merge candidates: keep newer, delete older
    //    - Obsolete: soft-delete
}
```

**Cost:** Haiku at ~$0.25/MTok input, ~$1.25/MTok output. A consolidation pass with 50 memories is roughly 2K input tokens + 500 output tokens = ~$0.001 per pass. At every 30 minutes for an 8-hour workday, that's $0.016/day. Negligible.

### 5. Context Generation (Session Start)

This is the critical path — it replaces the "load first 200 lines of MEMORY.md" mechanism. Called once per session via IPC.

```rust
async fn get_context(&self, max_tokens: usize) -> String {
    // 1. Gather high-importance, non-expired memories
    let memories = self.db.query(
        "SELECT * FROM memories
         WHERE (decay_at IS NULL OR decay_at > datetime('now'))
         ORDER BY importance DESC, updated_at DESC
         LIMIT 100"
    );

    // 2. Gather recent consolidation insights
    let insights = self.db.query(
        "SELECT * FROM consolidations ORDER BY created_at DESC LIMIT 20"
    );

    // 3. Partition by type
    let architecture = memories.filter(|m| m.memory_type == "architecture");
    let decisions    = memories.filter(|m| m.memory_type == "decision");
    let patterns     = memories.filter(|m| m.memory_type == "pattern");
    let gotchas      = memories.filter(|m| m.memory_type == "gotcha");
    let preferences  = memories.filter(|m| m.memory_type == "preference");
    let progress     = memories.filter(|m| m.memory_type == "progress");

    // 4. Build structured summary using summaries, not full content
    //    This is what gets injected into the system prompt.
    let mut output = String::from("# Project Memory\n\n");

    if !architecture.is_empty() {
        output += "## Architecture\n";
        for m in architecture.iter().take(10) {
            output += &format!("- {}\n", m.summary);
        }
        output += "\n";
    }

    // ... same for each type ...

    if !insights.is_empty() {
        output += "## Key Insights\n";
        for i in insights.iter().take(5) {
            output += &format!("- {}\n", i.insight);
        }
    }

    // 5. Token-check: if over budget, ask Haiku to compress further
    let estimated_tokens = output.len() / 4; // rough estimate
    if estimated_tokens > max_tokens {
        output = self.compress_with_haiku(&output, max_tokens).await;
    }

    output
}
```

**The key insight here:** The current system dumps 200 lines of raw, unprocessed notes. This system dumps ~1,500 tokens of Haiku-compressed, deduplicated, cross-referenced, importance-ranked summaries organized by type. Same token budget, dramatically higher information density.

### 6. Integration into Claude Code Core

Claude Code is a TypeScript/Node.js CLI app distributed via npm, with the main agent logic in a bundled JS file. The integration points are surgical:

**A. Binary distribution**

The `claude-memoryd` Rust binary ships alongside Claude Code, similar to how `esbuild`, `turbo`, and `swc` distribute platform-specific native binaries via npm:

```
@anthropic-ai/claude-code/
├── bin/
│   ├── claude           # existing CLI entry
│   └── claude-memoryd   # new: platform-specific native binary
├── optionalDependencies:
│   ├── @anthropic-ai/memoryd-darwin-arm64
│   ├── @anthropic-ai/memoryd-darwin-x64
│   ├── @anthropic-ai/memoryd-linux-x64
│   ├── @anthropic-ai/memoryd-linux-arm64
│   └── @anthropic-ai/memoryd-win32-x64
```

Each platform package contains the compiled binary. npm's `optionalDependencies` mechanism installs only the matching platform.

**B. Session startup (TypeScript side)**

In the existing session initialization code, where Claude Code currently reads MEMORY.md:

```typescript
// BEFORE (current implementation, simplified):
async function loadAutoMemory(projectPath: string): Promise<string> {
    const memoryPath = getMemoryDir(projectPath) + '/MEMORY.md';
    const content = await fs.readFile(memoryPath, 'utf-8');
    const lines = content.split('\n');
    if (lines.length > 200) {
        return lines.slice(0, 200).join('\n') +
            `\nWARNING: MEMORY.md is ${lines.length} lines (limit: 200).`;
    }
    return content;
}

// AFTER (new implementation):
async function loadAutoMemory(projectPath: string): Promise<string> {
    const daemon = await MemoryDaemon.ensure(projectPath);
    const context = await daemon.getContext({ maxTokens: 1500 });
    return context;
}
```

**C. Memory writes (TypeScript side)**

Where Claude currently writes to MEMORY.md via its file write tool:

```typescript
// BEFORE: Claude writes directly to MEMORY.md file
// (handled by the standard Write tool targeting the memory path)

// AFTER: Intercept writes to the memory directory
async function handleMemoryWrite(projectPath: string, content: string) {
    const daemon = await MemoryDaemon.ensure(projectPath);
    await daemon.ingest({ content, sessionId: currentSessionId });
    // No file write — the daemon owns the storage
}
```

**D. Session end**

```typescript
async function onSessionEnd(projectPath: string, sessionId: string) {
    const daemon = await MemoryDaemon.ensure(projectPath);
    // Send a final summary of what happened this session
    await daemon.endSession({ sessionId });
    // Daemon keeps running for consolidation; will idle-timeout later
}
```

**E. The `/memory` command**

Currently shows MEMORY.md and lets you edit it. Updated to show a formatted view of the structured memory store:

```typescript
async function handleMemoryCommand(projectPath: string) {
    const daemon = await MemoryDaemon.ensure(projectPath);
    const status = await daemon.getStatus();
    // Display:
    //   Active memories: 147
    //   Consolidation insights: 23
    //   Memory types: architecture(12) decision(8) pattern(31) gotcha(19) preference(14) progress(63)
    //   Last consolidation: 12 minutes ago
    //   Token budget: 1340/1500
    //
    //   [Toggle auto-memory on/off]
    //   [Open memory DB in viewer]
    //   [Export as markdown]
    //   [Reset memory]
}
```

---

## Migration Path

**Existing MEMORY.md → SQLite import:**

On first launch after upgrade, if `memory/MEMORY.md` exists:

1. Read the file
2. Send the entire content to Haiku as a single ingest: "Parse this collection of memory notes into individual structured memories"
3. Haiku returns a JSON array of structured memories
4. Insert all into SQLite
5. Rename `memory/MEMORY.md` → `memory/MEMORY.md.bak`
6. Write a marker file `memory/.migrated`

The user sees no change in behavior. Their existing memories are preserved and actually improved (now structured and searchable).

**Fallback:** If the daemon fails to start (binary missing, permissions, etc.), fall back to the current MEMORY.md behavior. The system degrades gracefully rather than breaking.

---

## What This Does NOT Change

- **CLAUDE.md files** — untouched. Still human-written, still loaded in full, still the authoritative instructions.
- **`.claude/rules/`** — untouched. Conditional rules still work exactly as before.
- **Session memory** — the conversation-level session recall is separate and unaffected.
- **Subagent memory** — agent-specific memory in `.claude/agent-memory/` is separate.
- **The system prompt structure** — the active memory summary occupies the same slot that MEMORY.md content currently occupies.
- **Token count** — the context injection stays at ~1,500 tokens. The improvement is in *information density*, not size.

---

## Consolidated Benefits

| Dimension | Current (MEMORY.md) | Active Memory |
|-----------|---------------------|---------------|
| Capacity | 200 lines hard cap | Unlimited (SQLite), budget-compressed at injection |
| Deduplication | None (same fact recorded many times) | Haiku dedup on ingest + consolidation merge |
| Cross-referencing | None | Consolidation loop finds connections every 30 min |
| Staleness | Entries sit forever | Typed decay: progress fades, architecture persists |
| Information density | Raw unprocessed notes | Haiku-compressed summaries ranked by importance |
| Search | None (grep the file) | FTS5 full-text search |
| Organization | Flat text | Typed (architecture/decision/pattern/gotcha/etc) |
| Token cost at startup | ~1,500 tokens of raw notes | ~1,500 tokens of compressed, prioritized summaries |
| Background processing | None | Continuous consolidation between sessions |
| API cost | $0 | ~$0.02/day with Haiku |

---

## Rust Project Structure

```
claude-memoryd/
├── Cargo.toml
├── src/
│   ├── main.rs                 # CLI entry, arg parsing, daemon setup
│   ├── daemon.rs               # Tokio runtime, IPC socket server, idle timeout
│   ├── db/
│   │   ├── mod.rs
│   │   ├── schema.rs           # Table creation, migrations
│   │   ├── memories.rs         # CRUD for memories table
│   │   ├── consolidations.rs   # CRUD for consolidations
│   │   └── fts.rs              # FTS5 index management
│   ├── ingest/
│   │   ├── mod.rs
│   │   ├── pipeline.rs         # Raw note → Haiku → structured memory
│   │   └── dedup.rs            # Jaccard similarity, merge logic
│   ├── consolidate/
│   │   ├── mod.rs
│   │   ├── loop.rs             # Timer-driven consolidation tick
│   │   └── decay.rs            # Expiry cleanup
│   ├── context/
│   │   ├── mod.rs
│   │   └── generator.rs        # Build session context from DB
│   ├── api/
│   │   ├── mod.rs
│   │   └── haiku.rs            # Anthropic API client (Haiku calls)
│   └── ipc/
│       ├── mod.rs
│       ├── protocol.rs         # JSON-over-socket message types
│       └── handler.rs          # Request dispatch
├── tests/
│   ├── ingest_test.rs
│   ├── consolidation_test.rs
│   ├── context_test.rs
│   └── integration_test.rs
└── build.rs                    # Cross-compilation setup
```

---

## Open Questions

1. **Multi-session concurrency.** If a user runs 3 parallel Claude Code sessions on the same project, all three share one daemon instance. The IPC protocol needs session-ID scoping so `get_context` can track what each session has already seen.

2. **Offline / air-gapped environments.** If the Anthropic API is unreachable, the daemon should degrade: store raw notes without Haiku processing, skip consolidation, and serve unprocessed summaries at session start. Process the backlog when connectivity returns.

3. **Memory export / portability.** Users should be able to `claude memory export` to get a markdown dump of their structured memory, and `claude memory import` to load one. This replaces the current "just edit the markdown file" workflow.

4. **Privacy.** All memory stays local (SQLite on disk, same as the current markdown files). Haiku API calls send project memory content to Anthropic's API — same trust boundary as the existing Claude Code session itself, since the user is already sending their code to Claude.

---

## Implementation Phases

**Phase 1: Daemon + Ingest (2-3 weeks)**
- Rust binary with SQLite, IPC socket, ingest pipeline
- Haiku integration for structured extraction
- Basic deduplication
- TypeScript-side: spawn daemon, route memory writes

**Phase 2: Context Generation (1 week)**
- Replace MEMORY.md loading with daemon query
- Importance-ranked, type-organized context builder
- Token budget management
- Migration logic for existing MEMORY.md files

**Phase 3: Consolidation (1-2 weeks)**
- Timer-driven consolidation loop
- Cross-reference detection
- Insight generation
- Decay/cleanup system

**Phase 4: Polish (1 week)**
- `/memory` command updated with rich status view
- Export/import
- Fallback to MEMORY.md if daemon fails
- Telemetry for memory system health
- Cross-platform binary builds + CI

---

## Summary

This is not a new feature — it's a direct replacement for the weakest part of Claude Code's existing memory system. The 200-line MEMORY.md with no processing is the bottleneck that causes context loss, repeated explanations, and project amnesia across sessions. The active memory daemon fixes this by doing what the always-on-memory-agent does: continuously ingest, consolidate, and serve compressed, cross-referenced knowledge — all for about two cents a day in Haiku API costs and zero additional tokens in the context window.

The user sees nothing new. They just notice that Claude remembers better. https://github.com/anthropics/claude-code
