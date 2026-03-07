# claude-code-remembers

Active memory daemon for Claude Code. Replaces the 200-line `MEMORY.md` with a SQLite-backed, Haiku-powered memory system that continuously ingests, deduplicates, consolidates, and serves compressed project knowledge.

## The Problem

Claude Code's auto-memory (`~/.claude/projects/<project>/memory/MEMORY.md`) is a flat file with a hard 200-line cap. No deduplication, no cross-referencing, no compression. Stale entries crowd out useful ones. Session 14's insight is never connected to session 3's architecture decision.

## What This Does

`claude-memoryd` is a background daemon that:

- **Ingests** raw memory notes and uses Haiku to extract structured metadata (type, importance, entities, topics)
- **Deduplicates** via FTS5 search + Jaccard similarity — the same fact won't be stored 5 times
- **Consolidates** every 30 minutes — finds connections between memories, generates cross-cutting insights, merges duplicates, removes obsolete entries
- **Serves** compressed, importance-ranked, type-organized context (~1,500 tokens) at session start
- **Decays** stale memories — progress notes expire in 7 days, preferences in 90 days, architecture decisions persist forever

All for ~$0.02/day in Haiku API costs.

## Architecture

```
Claude Code Session ←→ Unix Socket IPC ←→ claude-memoryd ←→ SQLite + Haiku API
```

The daemon runs as a background process, persists between sessions (2-hour idle timeout), and communicates via a Unix domain socket with a JSON-line protocol.

## Building

```bash
cargo build --release
# Binary at target/release/claude-memoryd (~7MB)
```

## Usage

### Standalone (without Claude Code modification)

#### 1. Start the daemon

```bash
# Set your API key for Haiku-powered extraction and consolidation
export ANTHROPIC_API_KEY="sk-ant-..."

# Start the daemon for a project
claude-memoryd \
  --project /path/to/your/project \
  --db ~/.claude/projects/your-project/memory.db \
  --socket ~/.claude/projects/your-project/memoryd.sock
```

The daemon runs in the foreground (use `&` or a process manager to background it). It will idle-timeout after 2 hours of no IPC activity.

Without `ANTHROPIC_API_KEY`, it runs in **offline mode** — stores raw notes without Haiku processing, skips consolidation.

#### 2. Ingest a memory

```bash
echo '{"method":"ingest","params":{"content":"The auth service uses JWT with RS256 and Redis for token blacklisting","session_id":"session-1"}}' \
  | nc -U ~/.claude/projects/your-project/memoryd.sock
```

Response:
```json
{"status":"ok","data":{"memory_id":1,"deduplicated":false}}
```

#### 3. Get session context

```bash
echo '{"method":"get_context","params":{"max_tokens":1500}}' \
  | nc -U ~/.claude/projects/your-project/memoryd.sock
```

Response:
```json
{"status":"ok","data":{"context":"# Project Memory\n\n## Architecture\n- Auth service uses JWT with RS256 and Redis for token blacklisting\n\n","token_estimate":25}}
```

#### 4. Check status

```bash
echo '{"method":"get_status","params":null}' \
  | nc -U ~/.claude/projects/your-project/memoryd.sock
```

#### 5. Search memories

```bash
echo '{"method":"search","params":{"query":"auth JWT","limit":5}}' \
  | nc -U ~/.claude/projects/your-project/memoryd.sock
```

### Using the helper script

A convenience wrapper for common operations:

```bash
# Start daemon for current directory
./scripts/memoryd.sh start

# Ingest a note
./scripts/memoryd.sh ingest "Redis cache TTL is set to 5 minutes for auth tokens"

# Get context (what would be injected into Claude's system prompt)
./scripts/memoryd.sh context

# Check status
./scripts/memoryd.sh status

# Stop daemon
./scripts/memoryd.sh stop
```

### Migrating existing MEMORY.md

```bash
# Pipe your existing MEMORY.md through the daemon
while IFS= read -r line; do
  [ -z "$line" ] && continue
  echo "{\"method\":\"ingest\",\"params\":{\"content\":\"$line\"}}" \
    | nc -U ~/.claude/projects/your-project/memoryd.sock
done < ~/.claude/projects/your-project/memory/MEMORY.md
```

## IPC Protocol

JSON-line protocol over Unix domain socket. Send one JSON object per line, receive one JSON response per line.

| Method | Params | Description |
|--------|--------|-------------|
| `ingest` | `{content, session_id?}` | Store a new memory note |
| `get_context` | `{max_tokens, session_id?}` | Get compressed context for system prompt injection |
| `get_status` | `null` | Memory counts, types, last consolidation time |
| `search` | `{query, limit?}` | FTS5 full-text search |
| `end_session` | `{session_id}` | Signal session end, triggers decay cleanup |

## Memory Types

| Type | Decay | Examples |
|------|-------|---------|
| `architecture` | Never | "Uses microservices with Redis caching" |
| `decision` | Never | "Chose PostgreSQL over MongoDB for ACID compliance" |
| `pattern` | Never | "All API endpoints follow REST conventions with /v1/ prefix" |
| `gotcha` | 90 days | "The test suite requires Docker running for integration tests" |
| `preference` | 90 days | "User prefers snake_case, wants no emojis" |
| `progress` | 7 days | "Finished implementing the auth middleware" |

## Integration with Claude Code

This daemon is designed to replace Claude Code's internal `MEMORY.md` system. Full integration requires modifying Claude Code's TypeScript source to:

1. Spawn `claude-memoryd` at session start
2. Route memory writes through IPC instead of file writes
3. Query the daemon for session context instead of reading `MEMORY.md`

See [CLAUDE.md](./CLAUDE.md) for the full RFC with integration design, TypeScript code samples, and implementation phases.

## Project Structure

```
src/
├── main.rs                          # CLI, tokio runtime, daemon lifecycle
├── daemon.rs                        # Shared state, IPC request handlers
├── api/haiku.rs                     # Anthropic Messages API client (Haiku)
├── db/
│   ├── schema.rs                    # SQLite table creation + FTS5
│   ├── memories.rs                  # Memory CRUD operations
│   ├── consolidations.rs            # Consolidation insights CRUD
│   └── fts.rs                       # Full-text search queries
├── ingest/
│   ├── pipeline.rs                  # Raw note → Haiku → structured memory
│   └── dedup.rs                     # Jaccard similarity deduplication
├── context/
│   └── generator.rs                 # Build ranked, typed context from DB
├── consolidate/
│   ├── consolidation_loop.rs        # Background consolidation with Haiku
│   └── decay.rs                     # Expiry cleanup
└── ipc/
    ├── protocol.rs                  # JSON message types
    └── handler.rs                   # Unix socket server
```

## License

MIT
