# claude-remember

MCP server that gives Claude Code active, structured project memory. Replaces the flat 200-line `MEMORY.md` with a SQLite-backed daemon that deduplicates, classifies, consolidates, and serves compressed context.

```
Claude Code --MCP--> claude-remember (SQLite + Haiku)
                       |
                       +-- memory_remember  -> ingest + classify + dedup
                       +-- memory_recall    -> FTS5 search
                       +-- memory_context   -> compressed structured context
                       +-- memory_status    -> stats
```

## Install

### Prerequisites

- [Rust](https://rustup.rs/) toolchain (`cargo`)
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed
- `ANTHROPIC_API_KEY` in your environment (optional — for Haiku-powered classification)

### One-line install

```bash
git clone https://github.com/agiprolabs/claude-code-remembers.git
cd claude-code-remembers
./install.sh
```

This builds the daemon, installs it to `~/.local/bin/`, and registers it as an MCP server in `~/.claude/settings.json`.

### Manual install

```bash
cargo build --release
cp target/release/claude-remember ~/.local/bin/

# Register with Claude Code
./scripts/setup-mcp.sh
```

## Usage

Once installed, Claude Code automatically has four new tools:

| Tool | What it does |
|------|-------------|
| `memory_remember` | Store a memory — the daemon classifies, deduplicates, and indexes it |
| `memory_recall` | Search memories by keyword or topic via FTS5 |
| `memory_context` | Get the full compressed context, organized by type |
| `memory_status` | Stats — total memories, counts by type, last consolidation |

### CLAUDE.md setup

Add this to your project's `CLAUDE.md` so Claude uses the memory tools:

```markdown
## Memory
At the start of each session, call memory_context to load project memories.
When you learn something important about the project, call memory_remember.
Use memory_recall to search for relevant past knowledge.
```

### Resource

The server also exposes a `memory://context` resource that can be loaded by Claude Code for auto-injection of project context.

## How it works

The daemon runs as an MCP server (stdio transport). Claude Code starts it automatically when a session begins and communicates via JSON-RPC.

### Memory lifecycle

1. **Ingest** — Claude calls `memory_remember` with an observation
2. **Classify** — Haiku categorizes it (architecture/decision/pattern/gotcha/preference/progress) and scores importance 0.0-1.0
3. **Deduplicate** — FTS5 search + Jaccard similarity finds and replaces duplicates
4. **Store** — SQLite with full-text search indexing
5. **Consolidate** — Every 30 minutes, Haiku finds connections, generates insights, merges duplicates
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

## Configuration

No configuration needed beyond the initial install. The daemon inherits `ANTHROPIC_API_KEY` from your environment.

| Env var | Purpose |
|---------|---------|
| `ANTHROPIC_API_KEY` | Haiku API calls for classification and consolidation (optional) |
| `ANTHROPIC_BASE_URL` | Custom API endpoint (enterprise proxies, etc.) |

Without an API key, memories are stored with default classification (`progress`, importance 0.5). Everything else works — search, dedup, context generation — just without Haiku-powered intelligence.

## Project structure

```
claude-code-remembers/
+-- install.sh                       # Build + install + register MCP
+-- scripts/
|   +-- setup-mcp.sh                # Register MCP server with Claude Code
|   +-- claude-remembers            # Shell wrapper (legacy, optional)
|   +-- remember.sh                 # Manual daemon management
+-- src/
|   +-- main.rs                     # CLI, tokio runtime, MCP/socket mode
|   +-- daemon.rs                   # Shared state, request handlers
|   +-- mcp/
|   |   +-- server.rs              # MCP JSON-RPC server (stdio)
|   +-- api/haiku.rs               # Anthropic Messages API client
|   +-- db/
|   |   +-- schema.rs              # SQLite tables + FTS5
|   |   +-- memories.rs            # Memory CRUD
|   |   +-- consolidations.rs      # Consolidation insights CRUD
|   |   +-- fts.rs                 # Full-text search
|   +-- ingest/
|   |   +-- pipeline.rs            # Raw note -> Haiku -> structured memory
|   |   +-- dedup.rs               # Jaccard similarity
|   +-- context/
|   |   +-- generator.rs           # Build ranked, typed context
|   +-- consolidate/
|   |   +-- consolidation_loop.rs  # Background consolidation
|   |   +-- decay.rs               # Expiry cleanup
|   +-- ipc/
|       +-- protocol.rs            # JSON message types
|       +-- handler.rs             # Unix socket server (legacy)
+-- Cargo.toml
+-- CLAUDE.md
```

## License

MIT
