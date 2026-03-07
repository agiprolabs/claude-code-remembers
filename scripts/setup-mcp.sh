#!/usr/bin/env bash
set -euo pipefail

# Setup claude-memoryd as an MCP server for Claude Code
# This registers the daemon in Claude Code's MCP settings and configures hooks.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# --- Find binary ---

if command -v claude-memoryd &>/dev/null; then
    MEMORYD_BIN="$(command -v claude-memoryd)"
elif [ -f "$PROJECT_ROOT/target/release/claude-memoryd" ]; then
    MEMORYD_BIN="$PROJECT_ROOT/target/release/claude-memoryd"
else
    echo "Error: claude-memoryd not found. Build first: cargo build --release"
    exit 1
fi

echo "Found claude-memoryd at: $MEMORYD_BIN"

# --- Determine scope ---

SCOPE="${1:-user}"

if [ "$SCOPE" = "project" ]; then
    SETTINGS_DIR=".claude"
    SETTINGS_FILE="$SETTINGS_DIR/settings.json"
    echo "Installing for current project: $(pwd)"
else
    SETTINGS_DIR="$HOME/.claude"
    SETTINGS_FILE="$SETTINGS_DIR/settings.json"
    echo "Installing globally for user"
fi

mkdir -p "$SETTINGS_DIR"

# --- Compute DB path ---

PROJECT_DIR="$(pwd)"
ENCODED_PROJECT=$(echo "$PROJECT_DIR" | sed 's|[^a-zA-Z0-9]|-|g')
DB_DIR="$HOME/.claude/projects/$ENCODED_PROJECT/memoryd"
DB_PATH="$DB_DIR/memory.db"
mkdir -p "$DB_DIR"

# --- Update settings.json ---

if [ -f "$SETTINGS_FILE" ]; then
    SETTINGS=$(cat "$SETTINGS_FILE")
else
    SETTINGS="{}"
fi

# Add MCP server config using python3 for reliable JSON manipulation
python3 -c "
import json, sys

settings = json.loads('''$SETTINGS''')

if 'mcpServers' not in settings:
    settings['mcpServers'] = {}

settings['mcpServers']['claude-memoryd'] = {
    'command': '$MEMORYD_BIN',
    'args': [
        '--project', '$PROJECT_DIR',
        '--db', '$DB_PATH',
        '--mcp'
    ]
}

with open('$SETTINGS_FILE', 'w') as f:
    json.dump(settings, f, indent=2)
    f.write('\n')
"

echo "MCP server registered in $SETTINGS_FILE"

# --- Show result ---

echo ""
echo "=== Setup complete ==="
echo ""
echo "Claude Code will now start claude-memoryd as an MCP server."
echo "Available tools:"
echo "  - memory_remember: Store a memory"
echo "  - memory_recall:   Search memories"
echo "  - memory_context:  Get full compressed context"
echo "  - memory_status:   Check memory system status"
echo ""
echo "Available resources:"
echo "  - memory://context: Auto-loadable project memory"
echo ""
echo "Database: $DB_PATH"
echo ""
echo "To add a CLAUDE.md instruction telling Claude to use memories:"
echo "  cat >> CLAUDE.md << 'EOF'"
echo ""
echo "## Memory"
echo "At the start of each session, call memory_context to load project memories."
echo "When you learn something important about the project, call memory_remember."
echo "Use memory_recall to search for relevant past knowledge."
echo "EOF"
echo ""
echo "To uninstall: $0 uninstall"
