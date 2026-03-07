#!/usr/bin/env bash
set -euo pipefail

# Install claude-remembers
# Builds the Rust daemon and installs both binaries to ~/.local/bin

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

echo "=== claude-code-remembers installer ==="
echo ""

# --- Prerequisites ---

# Check for Rust
if ! command -v cargo &>/dev/null; then
    echo "Error: Rust not found. Install it:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

# Check for Claude Code
if ! command -v claude &>/dev/null; then
    echo "Error: Claude Code not found. Install it:"
    echo "  npm install -g @anthropic-ai/claude-code"
    echo ""
    echo "Or if using the native binary:"
    echo "  See https://docs.anthropic.com/en/docs/claude-code"
    exit 1
fi

echo "Found claude at: $(command -v claude)"
echo "Found cargo at: $(command -v cargo)"
echo ""

# --- Build ---

echo "Building claude-memoryd (release mode)..."
cd "$SCRIPT_DIR"
cargo build --release

echo ""

# --- Install ---

mkdir -p "$INSTALL_DIR"

echo "Installing to $INSTALL_DIR..."

# Install the daemon binary
cp "$SCRIPT_DIR/target/release/claude-memoryd" "$INSTALL_DIR/claude-memoryd"
echo "  Installed: claude-memoryd"

# Install the wrapper script
cp "$SCRIPT_DIR/scripts/claude-remembers" "$INSTALL_DIR/claude-remembers"
chmod +x "$INSTALL_DIR/claude-remembers"
echo "  Installed: claude-remembers"

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

# --- Verify ---

echo "=== Installation complete ==="
echo ""
echo "Usage (drop-in replacement for claude):"
echo ""
echo "  claude-remembers                              # interactive session"
echo "  claude-remembers --dangerously-skip-permissions  # skip permissions"
echo "  claude-remembers --resume                     # resume last session"
echo "  claude-remembers -p \"explain this code\"       # non-interactive"
echo ""
echo "All claude flags and arguments work exactly the same."
echo ""
echo "What's different:"
echo "  - A background daemon (claude-memoryd) manages your project memory"
echo "  - Memories are deduplicated, typed, and importance-ranked"
echo "  - Context injected into MEMORY.md is compressed and organized"
echo "  - Between sessions, memories are consolidated and cross-referenced"
echo ""
echo "Set ANTHROPIC_API_KEY for Haiku-powered memory processing (~\$0.02/day)."
echo "Without it, memories are stored but not processed (offline mode)."
echo ""
echo "Compatible with all existing Claude Code projects and settings."
echo "Your CLAUDE.md, .claude/rules/, and settings are untouched."
