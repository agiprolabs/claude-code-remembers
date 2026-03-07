# claude-code-remembers

Drop-in replacement for the `claude` command with active memory. Use `claude-remembers` exactly like `claude` — same flags, same projects, same config — but with a background daemon that makes Claude's memory actually work.

```bash
# Instead of this:
claude --dangerously-skip-permissions --resume

# Use this:
claude-remembers --dangerously-skip-permissions --resume
```

## What's different

Claude Code's auto-memory (`MEMORY.md`) is a flat file with a 200-line cap. No deduplication, no compression, no processing. `claude-remembers` adds a background daemon that:

- **Structures** memories by type (architecture, decision, pattern, gotcha, preference, progress)
- **Deduplicates** via FTS5 search + Jaccard similarity
- **Consolidates** every 30 minutes — finds connections, generates insights, merges duplicates
- **Ranks** by importance and serves compressed context (~1,500 tokens, same budget)
- **Decays** stale entries — progress fades in 7 days, architecture persists forever
- **Uses Haiku** for cheap background processing (~$0.02/day)

Your existing CLAUDE.md, `.claude/rules/`, settings, and projects are untouched.

## Install

### Prerequisites

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed and working (`claude` command available)
- [Rust](https://rustup.rs/) toolchain (`cargo`)
- `ANTHROPIC_API_KEY` set in your environment (for Haiku-powered processing; works without it in offline mode)

### One-line install

```bash
git clone https://github.com/agiprolabs/claude-code-remembers.git
cd claude-code-remembers
./install.sh
```

This builds the Rust daemon and installs two binaries to `~/.local/bin/`:
- `claude-memoryd` — the background memory daemon
- `claude-remembers` — the CLI wrapper

### Manual install

```bash
git clone https://github.com/agiprolabs/claude-code-remembers.git
cd claude-code-remembers

# Build the daemon
cargo build --release

# Copy binaries to your PATH
cp target/release/claude-memoryd ~/.local/bin/
cp scripts/claude-remembers ~/.local/bin/
chmod +x ~/.local/bin/claude-remembers

# Make sure ~/.local/bin is in your PATH
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

### Verify

```bash
claude-remembers --version
# Should show the same version as `claude --version`
```

## Usage

Use `claude-remembers` exactly like `claude`. All flags and arguments are passed through:

```bash
# Interactive session
claude-remembers

# Skip permissions
claude-remembers --dangerously-skip-permissions

# Resume last session
claude-remembers --resume

# Non-interactive
claude-remembers -p "explain this function"

# Combine flags
claude-remembers --dangerously-skip-permissions --resume

# With a specific model
claude-remembers --model claude-sonnet-4-20250514
```

### Existing projects

`claude-remembers` is compatible with all existing Claude Code projects. It reads and writes the same `MEMORY.md` that Claude Code uses. The daemon runs alongside — it doesn't replace any files or settings.

First time you run `claude-remembers` in a project that already has a `MEMORY.md`, the daemon will ingest those existing memories into its SQLite store. From then on, it serves compressed, organized context instead of raw notes.

### Memory daemon management

The daemon starts automatically when you run `claude-remembers` and idles out after 2 hours. You can also manage it manually:

```bash
# Check daemon status
./scripts/memoryd.sh status

# Manually ingest a memory
./scripts/memoryd.sh ingest "The API uses rate limiting with a 100 req/min window"

# Search memories
./scripts/memoryd.sh search "rate limiting"

# Get the context that would be injected
./scripts/memoryd.sh context

# Stop the daemon
./scripts/memoryd.sh stop

# Migrate an existing MEMORY.md into the daemon
./scripts/memoryd.sh migrate ~/.claude/projects/my-project/memory/MEMORY.md
```

## How it works

```
┌─────────────────────────────────────────────────────┐
│                 claude-remembers                     │
│                                                     │
│  1. Start claude-memoryd (if not running)            │
│  2. Query daemon → write compressed context          │
│     to MEMORY.md                                    │
│  3. Snapshot MEMORY.md                              │
│  4. Run `claude` with all your arguments            │
│  5. Diff MEMORY.md → ingest new entries into daemon  │
└──────────────────┬──────────────────────────────────┘
                   │
        ┌──────────▼──────────┐
        │   claude-memoryd     │
        │                     │
        │  SQLite + Haiku     │
        │  Ingest → Dedup →   │
        │  Consolidate →      │
        │  Serve context      │
        └─────────────────────┘
```

The wrapper uses MEMORY.md as a sync point:
- **Before session:** daemon writes organized context into MEMORY.md
- **During session:** Claude reads MEMORY.md (as normal) and writes new entries to it
- **After session:** wrapper diffs MEMORY.md against the pre-session snapshot, ingests new entries into the daemon

No Claude Code internals are modified.

## Memory types

| Type | Decay | Examples |
|------|-------|---------|
| `architecture` | Never | "Uses microservices with Redis caching" |
| `decision` | Never | "Chose PostgreSQL over MongoDB for ACID compliance" |
| `pattern` | Never | "All API endpoints follow REST conventions with /v1/ prefix" |
| `gotcha` | 90 days | "The test suite requires Docker running for integration tests" |
| `preference` | 90 days | "User prefers snake_case, wants no emojis" |
| `progress` | 7 days | "Finished implementing the auth middleware" |

Haiku classifies each memory into a type and assigns an importance score (0.0–1.0). Without an API key, all memories default to `progress` with 0.5 importance.

## Configuration

No configuration needed. The daemon inherits `ANTHROPIC_API_KEY` from your environment (the same key Claude Code uses).

| Env var | Purpose |
|---------|---------|
| `ANTHROPIC_API_KEY` | Haiku API calls for memory processing (optional, degrades gracefully) |
| `ANTHROPIC_BASE_URL` | Custom API endpoint (enterprise proxies, etc.) |
| `CLAUDE_CODE_DISABLE_AUTO_MEMORY` | Set to `1` to disable (respects Claude Code's setting) |

## Project structure

```
claude-code-remembers/
├── install.sh                       # One-line installer
├── scripts/
│   ├── claude-remembers             # CLI wrapper (the main entry point)
│   └── memoryd.sh                   # Daemon management helper
├── src/
│   ├── main.rs                      # Daemon CLI, tokio runtime, lifecycle
│   ├── daemon.rs                    # Shared state, IPC request handlers
│   ├── api/haiku.rs                 # Anthropic Messages API client
│   ├── db/
│   │   ├── schema.rs                # SQLite tables + FTS5
│   │   ├── memories.rs              # Memory CRUD
│   │   ├── consolidations.rs        # Consolidation insights CRUD
│   │   └── fts.rs                   # Full-text search
│   ├── ingest/
│   │   ├── pipeline.rs              # Raw note → Haiku → structured memory
│   │   └── dedup.rs                 # Jaccard similarity
│   ├── context/
│   │   └── generator.rs             # Build ranked, typed context
│   ├── consolidate/
│   │   ├── consolidation_loop.rs    # Background consolidation
│   │   └── decay.rs                 # Expiry cleanup
│   └── ipc/
│       ├── protocol.rs              # JSON message types
│       └── handler.rs               # Unix socket server
├── Cargo.toml
└── CLAUDE.md                        # Full RFC / design doc
```

## License

MIT
