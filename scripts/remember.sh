#!/usr/bin/env bash
set -euo pipefail

# Helper script for claude-remember
# Usage: remember.sh <command> [args]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${SCRIPT_DIR}/../target/release/claude-remember"
PROJECT_DIR="${REMEMBER_PROJECT:-$(pwd)}"

# Derive a safe directory name from the project path
PROJECT_HASH=$(echo -n "$PROJECT_DIR" | shasum -a 256 | cut -c1-16)
REMEMBER_DIR="${HOME}/.claude/remember/${PROJECT_HASH}"
DB_PATH="${REMEMBER_DIR}/memory.db"
SOCK_PATH="${REMEMBER_DIR}/remember.sock"
PID_PATH="${REMEMBER_DIR}/remember.sock.pid"

ensure_dir() {
    mkdir -p "$REMEMBER_DIR"
}

is_running() {
    if [ -f "$PID_PATH" ]; then
        local pid
        pid=$(cat "$PID_PATH")
        if kill -0 "$pid" 2>/dev/null; then
            return 0
        fi
    fi
    return 1
}

cmd_start() {
    ensure_dir

    if is_running; then
        echo "Daemon already running (pid $(cat "$PID_PATH"))"
        return 0
    fi

    if [ ! -f "$BINARY" ]; then
        echo "Binary not found at $BINARY"
        echo "Run: cargo build --release"
        exit 1
    fi

    echo "Starting claude-remember for: $PROJECT_DIR"
    "$BINARY" \
        --project "$PROJECT_DIR" \
        --db "$DB_PATH" \
        --socket "$SOCK_PATH" \
        &

    # Wait for socket
    for i in $(seq 1 20); do
        if [ -S "$SOCK_PATH" ]; then
            echo "Daemon started (pid $(cat "$PID_PATH"))"
            return 0
        fi
        sleep 0.25
    done

    echo "Daemon failed to start within 5 seconds"
    exit 1
}

cmd_stop() {
    if ! is_running; then
        echo "Daemon not running"
        return 0
    fi

    local pid
    pid=$(cat "$PID_PATH")
    echo "Stopping daemon (pid $pid)"
    kill "$pid"
    rm -f "$SOCK_PATH" "$PID_PATH"
}

cmd_status() {
    if ! is_running; then
        echo "Daemon not running"
        exit 1
    fi

    echo '{"method":"get_status","params":null}' | nc -U "$SOCK_PATH"
}

cmd_ingest() {
    local content="$1"

    if ! is_running; then
        echo "Daemon not running. Start it with: $0 start"
        exit 1
    fi

    # Escape content for JSON
    local escaped
    escaped=$(printf '%s' "$content" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')

    echo "{\"method\":\"ingest\",\"params\":{\"content\":${escaped}}}" | nc -U "$SOCK_PATH"
}

cmd_context() {
    local max_tokens="${1:-1500}"

    if ! is_running; then
        echo "Daemon not running. Start it with: $0 start"
        exit 1
    fi

    echo "{\"method\":\"get_context\",\"params\":{\"max_tokens\":${max_tokens}}}" | nc -U "$SOCK_PATH"
}

cmd_search() {
    local query="$1"

    if ! is_running; then
        echo "Daemon not running. Start it with: $0 start"
        exit 1
    fi

    local escaped
    escaped=$(printf '%s' "$query" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')

    echo "{\"method\":\"search\",\"params\":{\"query\":${escaped}}}" | nc -U "$SOCK_PATH"
}

cmd_migrate() {
    local memory_file="${1:-}"

    if [ -z "$memory_file" ]; then
        # Try to find the project's MEMORY.md
        memory_file="${HOME}/.claude/projects/$(echo -n "$PROJECT_DIR" | sed 's|/|-|g')/memory/MEMORY.md"
        if [ ! -f "$memory_file" ]; then
            echo "No MEMORY.md found. Specify path: $0 migrate /path/to/MEMORY.md"
            exit 1
        fi
    fi

    if ! is_running; then
        echo "Starting daemon first..."
        cmd_start
    fi

    echo "Migrating: $memory_file"
    local count=0
    while IFS= read -r line; do
        [ -z "$line" ] && continue
        [[ "$line" =~ ^#.*$ ]] && continue  # Skip markdown headings
        [[ "$line" =~ ^---$ ]] && continue  # Skip horizontal rules

        local escaped
        escaped=$(printf '%s' "$line" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')

        echo "{\"method\":\"ingest\",\"params\":{\"content\":${escaped}}}" | nc -U "$SOCK_PATH" > /dev/null
        count=$((count + 1))
    done < "$memory_file"

    echo "Migrated $count entries"
}

# Main dispatch
case "${1:-help}" in
    start)   cmd_start ;;
    stop)    cmd_stop ;;
    status)  cmd_status ;;
    ingest)  cmd_ingest "${2:?Usage: $0 ingest \"memory note\"}" ;;
    context) cmd_context "${2:-1500}" ;;
    search)  cmd_search "${2:?Usage: $0 search \"query\"}" ;;
    migrate) cmd_migrate "${2:-}" ;;
    help|*)
        echo "Usage: $0 <command> [args]"
        echo ""
        echo "Commands:"
        echo "  start              Start the memory daemon for the current project"
        echo "  stop               Stop the daemon"
        echo "  status             Show memory statistics"
        echo "  ingest <note>      Store a memory note"
        echo "  context [tokens]   Get compressed context (default: 1500 tokens)"
        echo "  search <query>     Full-text search memories"
        echo "  migrate [file]     Import existing MEMORY.md"
        echo ""
        echo "Environment:"
        echo "  ANTHROPIC_API_KEY  Required for Haiku-powered extraction/consolidation"
        echo "  REMEMBER_PROJECT   Override project directory (default: pwd)"
        ;;
esac
