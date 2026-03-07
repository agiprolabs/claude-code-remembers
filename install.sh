#!/usr/bin/env bash
set -euo pipefail

# Install claude-remember — active memory MCP server for Claude Code

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

echo "=== claude-remember installer ==="
echo ""

# --- Prerequisites ---

if ! command -v cargo &>/dev/null; then
    echo "Error: Rust not found. Install it:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

if ! command -v claude &>/dev/null; then
    echo "Warning: Claude Code not found. Install it before using the MCP server."
    echo "  See https://docs.anthropic.com/en/docs/claude-code"
    echo ""
fi

echo "Found cargo at: $(command -v cargo)"
echo ""

# --- Build ---

echo "Building claude-remember (release mode)..."
cd "$SCRIPT_DIR"
cargo build --release

echo ""

# --- Install binary ---

mkdir -p "$INSTALL_DIR"

cp "$SCRIPT_DIR/target/release/claude-remember" "$INSTALL_DIR/claude-remember"
echo "Installed: $INSTALL_DIR/claude-remember"

echo ""

# --- Verify PATH ---

if ! echo "$PATH" | grep -q "$INSTALL_DIR"; then
    echo "Warning: $INSTALL_DIR is not in your PATH."
    echo "Add it to your shell profile:"
    echo ""
    echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.zshrc"
    echo "  source ~/.zshrc"
    echo ""
fi

# --- Register MCP server ---

echo "Registering as Claude Code MCP server..."

REMEMBER_BIN="$INSTALL_DIR/claude-remember"
SETTINGS_FILE="$HOME/.claude/settings.json"
mkdir -p "$HOME/.claude"

if [ -f "$SETTINGS_FILE" ]; then
    SETTINGS=$(cat "$SETTINGS_FILE")
else
    SETTINGS="{}"
fi

python3 -c "
import json

settings = json.loads('''$SETTINGS''')

if 'mcpServers' not in settings:
    settings['mcpServers'] = {}

settings['mcpServers']['claude-remember'] = {
    'command': '$REMEMBER_BIN',
    'args': ['--project', '.', '--db', '.claude-remember/memory.db', '--mcp']
}

with open('$SETTINGS_FILE', 'w') as f:
    json.dump(settings, f, indent=2)
    f.write('\n')
"

echo "MCP server registered in $SETTINGS_FILE"

echo ""
echo "=== Installation complete ==="
echo ""
echo "Claude Code will now have these memory tools available:"
echo "  - memory_remember  Store a memory about the project"
echo "  - memory_recall    Search past memories"
echo "  - memory_context   Get full structured memory context"
echo "  - memory_status    Check memory system stats"
echo ""
echo "Add this to your project's CLAUDE.md for best results:"
echo ""
echo '  ## Memory'
echo '  At the start of each session, call memory_context to load project memories.'
echo '  When you learn something important about the project, call memory_remember.'
echo '  Use memory_recall to search for relevant past knowledge.'
echo ""
echo "Set ANTHROPIC_API_KEY for Haiku-powered memory processing (~\$0.02/day)."
echo "Without it, memories are stored but not classified (offline mode)."
