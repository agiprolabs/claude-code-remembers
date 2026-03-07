#!/bin/bash
# Startup script for claude-remember MCP server
# Resolves project path and DB location dynamically

BINARY_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY="$BINARY_DIR/target/release/claude-remember"

# Build if binary doesn't exist
if [ ! -f "$BINARY" ]; then
    echo "Building claude-remember..." >&2
    cargo build --release --manifest-path "$BINARY_DIR/Cargo.toml" >&2
fi

# Project path: use argument, or fall back to current directory
PROJECT="${1:-$(pwd)}"
PROJECT="$(cd "$PROJECT" 2>/dev/null && pwd || echo "$PROJECT")"

# Store DB per-project under ~/.claude/memory/
DB_DIR="$HOME/.claude/memory"
mkdir -p "$DB_DIR"

# Create a safe filename from the project path
DB_NAME="$(echo "$PROJECT" | sed 's|/|__|g' | sed 's|^__||')"
DB_PATH="$DB_DIR/${DB_NAME}.db"

# Consolidation interval: configurable via env var (default: 1800s = 30 min)
CONSOLIDATION_INTERVAL="${CONSOLIDATION_INTERVAL:-1800}"

exec "$BINARY" --mcp --project "$PROJECT" --db "$DB_PATH" --consolidation-interval "$CONSOLIDATION_INTERVAL"
