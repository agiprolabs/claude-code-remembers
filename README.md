# claude-remember

MCP server that gives Claude Code active, structured project memory. Replaces the flat 200-line `MEMORY.md` with a SQLite-backed daemon that deduplicates, classifies, consolidates, and serves compressed context — all powered by Haiku for about $0.02/day.

```
Claude Code --MCP--> claude-remember (SQLite + Haiku)
                       |
                       +-- memory_remember     -> ingest + classify + dedup
                       +-- memory_recall       -> FTS5 search
                       +-- memory_context      -> compressed structured context
                       +-- memory_status       -> system stats
                       +-- memory_session_end  -> extract memories from session summary
                       +-- memory_consolidate  -> manual consolidation trigger
                       +-- memory_feedback     -> rate recall results
                       +-- memory_configure    -> runtime settings
                       +-- memory_list         -> browse all memories
                       +-- memory_delete       -> remove a memory
                       +-- memory_update       -> edit existing memory
                       +-- memory_export       -> backup as JSON
                       +-- memory_import       -> restore from JSON
                       +-- memory_setup        -> generate CLAUDE.md snippet
```

## Install

### Prerequisites

- [Rust](https://rustup.rs/) toolchain (`cargo`)
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed
- `ANTHROPIC_API_KEY` in your environment (for Haiku-powered classification/consolidation)

### Quick start

```bash
git clone https://github.com/agiprolabs/claude-code-remember.git
cd claude-code-remember
cargo build --release

# Register with Claude Code
claude mcp add -s user claude-remember \
  -e ANTHROPIC_API_KEY="your-key-here" \
  -- ./start-remember.sh
```

### One-line install

```bash
git clone https://github.com/agiprolabs/claude-code-remember.git
cd claude-code-remember
./install.sh
```

## Usage

Once installed, Claude Code automatically has 14 MCP tools available.

### Core tools

| Tool | What it does |
|------|-------------|
| `memory_remember` | Store a memory — classifies, deduplicates, and indexes via Haiku |
| `memory_recall` | Search memories by keyword/topic via FTS5 full-text search |
| `memory_context` | Get full compressed context, organized by type (architecture/decisions/patterns/etc) |
| `memory_status` | Stats — memory counts by type, last consolidation, interval setting |

### Session lifecycle

| Tool | What it does |
|------|-------------|
| `memory_session_end` | Submit session summary — Haiku extracts multiple memories automatically |
| `memory_feedback` | Rate recalled memories as helpful/not to tune importance scores |

### Management

| Tool | What it does |
|------|-------------|
| `memory_list` | Browse all memories, optionally filter by type |
| `memory_delete` | Remove a memory by ID |
| `memory_update` | Edit an existing memory's content (re-summarizes via Haiku) |
| `memory_export` | Export all memories as JSON for backup/portability |
| `memory_import` | Import memories from JSON array (full pipeline processing) |

### System

| Tool | What it does |
|------|-------------|
| `memory_consolidate` | Manually trigger a consolidation pass |
| `memory_configure` | Change runtime settings (e.g. consolidation interval) |
| `memory_setup` | Generate the CLAUDE.md snippet for any project |

### CLAUDE.md setup

Add this to your project's `CLAUDE.md` so Claude uses the memory tools automatically:

```markdown
# Memory: claude-remember MCP

Use the `claude-remember` MCP tools every session:
- **Start**: Call `memory_context` to load project memory before doing work
- **During**: Call `memory_remember` to store decisions, patterns, gotchas, architecture
- **Search**: Call `memory_recall` to find relevant memories by keyword/topic
- **Rate**: Call `memory_feedback` after recall to mark memories helpful (true) or not (false)
- **End**: Call `memory_session_end` with a summary of what was accomplished
- **Browse**: Call `memory_list` to see all memories, optionally filter by type
- **Edit**: Call `memory_update` to change a memory, `memory_delete` to remove one
- **Maintain**: Call `memory_consolidate` to trigger consolidation, `memory_configure` to change settings
- **Portable**: Call `memory_export` to backup, `memory_import` to restore
- **Setup**: Call `memory_setup` to generate this snippet for new projects
- **Check**: Call `memory_status` to view memory system health and stats
```

Or just ask Claude to call `memory_setup` and it will generate this for you.

### Resource

The server also exposes a `memory://context` resource for auto-injection of project context.

## How it works

The daemon runs as an MCP server (stdio transport). Claude Code starts it automatically when a session begins and communicates via JSON-RPC.

### Memory lifecycle

1. **Ingest** — Claude calls `memory_remember` with an observation
2. **Classify** — Haiku categorizes it (architecture/decision/pattern/gotcha/preference/progress), scores importance 0.0-1.0, and generates semantic tags for keyword-independent search
3. **Deduplicate** — FTS5 search + Jaccard similarity finds and replaces duplicates
4. **Store** — SQLite with full-text search indexing
5. **Consolidate** — Periodically (configurable, default 30 min), Haiku finds connections, generates cross-cutting insights, merges duplicates, and removes obsolete entries
6. **Decay** — Progress fades in 7 days, gotchas/preferences in 90 days, architecture/decisions persist forever
7. **Serve** — `memory_context` returns a compressed, structured summary within token budget

### Memory types

| Type | Decay | Examples |
|------|-------|---------|
| `architecture` | Never | "Uses microservices with Redis caching" |
| `decision` | Never | "Chose PostgreSQL over MongoDB for ACID compliance" |
| `pattern` | Never | "All API endpoints follow REST conventions with /v1/ prefix" |
| `gotcha` | 90 days | "The test suite requires Docker running for integration tests" |
| `preference` | 90 days | "User prefers snake_case, wants no emojis" |
| `progress` | 7 days | "Finished implementing the auth middleware" |

### Global memory

Memories flagged as `is_global` by Haiku (user preferences, universal patterns) are automatically synced to a shared database at `~/.claude/memory/global.db`. These flow between all projects during consolidation, so preferences like "user prefers no emojis" follow you everywhere.

### Feedback loop

When you recall memories, rate them with `memory_feedback`. Helpful memories get boosted (+0.1 importance), unhelpful ones get reduced (-0.1). Over time, the most useful knowledge surfaces first in `memory_context`.

## Configuration

| Setting | How to set | Default |
|---------|-----------|---------|
| API key | `ANTHROPIC_API_KEY` env var | Required for Haiku features |
| API endpoint | `ANTHROPIC_BASE_URL` env var | `https://api.anthropic.com` |
| Consolidation interval | `CONSOLIDATION_INTERVAL` env var or `memory_configure` tool | 1800s (30 min) |

Without an API key, memories are stored with default classification (`progress`, importance 0.5). Search, dedup, and context generation still work — just without Haiku-powered intelligence.

### Runtime configuration

Change settings mid-session without restart:

```
"Set consolidation interval to 5 minutes" → memory_configure(consolidation_interval_secs: 300)
```

## Data storage

| Path | Purpose |
|------|---------|
| `~/.claude/memory/<project>.db` | Per-project SQLite database |
| `~/.claude/memory/global.db` | Shared global memories across all projects |

## Project structure

```
claude-code-remember/
├── start-remember.sh                # MCP startup script
├── install.sh                       # Build + install + register MCP
├── scripts/
│   ├── setup-mcp.sh                # Register MCP server with Claude Code
│   └── remember.sh                 # Manual daemon management
├── src/
│   ├── main.rs                     # CLI, tokio runtime, MCP/socket mode
│   ├── daemon.rs                   # Shared state, request handlers
│   ├── mcp/
│   │   └── server.rs              # MCP JSON-RPC server (stdio), 14 tools
│   ├── api/haiku.rs               # Anthropic Messages API client
│   ├── db/
│   │   ├── schema.rs              # SQLite tables + FTS5 + migrations
│   │   ├── memories.rs            # Memory CRUD (with semantic tags, global flag)
│   │   ├── consolidations.rs      # Consolidation insights CRUD
│   │   └── fts.rs                 # Full-text search
│   ├── ingest/
│   │   ├── pipeline.rs            # Raw note -> Haiku -> structured memory
│   │   └── dedup.rs               # Jaccard similarity
│   ├── context/
│   │   └── generator.rs           # Build ranked, typed context
│   ├── consolidate/
│   │   ├── consolidation_loop.rs  # Background consolidation
│   │   └── decay.rs               # Expiry cleanup
│   └── ipc/
│       ├── protocol.rs            # JSON message types
│       └── handler.rs             # Unix socket server (legacy)
├── Cargo.toml
└── CLAUDE.md
```

## License

MIT
