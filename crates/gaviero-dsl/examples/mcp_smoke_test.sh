#!/usr/bin/env bash
#
# Quick check that an external MCP server works for every gaviero client.
#
# Runs the mcp_smoke_test.gaviero probe against the server you pass on the
# command line and STOPS with a non-zero exit as soon as any client (Claude,
# Codex, Cursor) cannot reach or use it. Each client writes a one-line verdict
# file (MCP_OK / MCP_FAIL); this wrapper is the gate, because gaviero-cli's own
# exit code only catches backend failures (e.g. Codex's required-server check),
# not "Claude/Cursor finished but the tool returned nothing".
#
# Usage:
#   mcp_smoke_test.sh <server-name> <url>
#   mcp_smoke_test.sh <server-name> --stdio <command> [args...]
#
# Examples:
#   mcp_smoke_test.sh semantic-scholar https://YOUR-ENDPOINT/
#   mcp_smoke_test.sh my-fs --stdio npx -y @modelcontextprotocol/server-filesystem /tmp
#
# Env knobs:
#   OUT_DIR=/path   scratch workspace for verdict files (default: a mktemp dir)
#   CLIENTS="claude codex cursor"   which providers to probe, in order
#   MODE=fastfail   stop at the first failing client (default)
#   MODE=all        probe every client in one run, then report all failures
#
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$here/mcp_smoke_test.gaviero"

usage() { sed -n '2,30p' "$0"; exit 2; }
[ $# -ge 2 ] || usage

name="$1"; shift

# Build the MCP flag (remote URL or local stdio command).
mcp_flag=()
if [ "${1:-}" = "--stdio" ]; then
    shift
    [ $# -ge 1 ] || { echo "error: --stdio needs a command" >&2; exit 2; }
    joined="$1"; shift
    for a in "$@"; do joined="$joined,$a"; done
    mcp_flag=(--mcp-stdio "$name=$joined")
else
    mcp_flag=(--mcp-url "$name=$1"); shift
fi

OUT_DIR="${OUT_DIR:-$(mktemp -d)}"
CLIENTS="${CLIENTS:-claude codex cursor}"
MODE="${MODE:-fastfail}"
mkdir -p "$OUT_DIR"

echo "[mcp-smoke] server=$name  workspace=$OUT_DIR  mode=$MODE"

run_probe() {  # $1 = workflow name (mcp-smoke-test | probe-<client>)
    gaviero-cli --script "$script" --workflow "$1" \
        --workspace "$OUT_DIR" --var OUT_DIR=. --var MCP_SERVER="$name" \
        --mcp-codex-trust granted \
        "${mcp_flag[@]}"
    # Ignore the exit status on purpose — we gate on the verdict files so the
    # check behaves the same across all three providers.
    return 0
}

verdict_ok() {  # $1 = client; 0 = MCP_OK, 1 = missing/empty/MCP_FAIL
    local f="$OUT_DIR/probe_$1.probe"
    [ -s "$f" ] && head -n 1 "$f" | grep -q '^MCP_OK'
}

report() {  # $1 = client, $2 = OK|FAIL
    if [ "$2" = OK ]; then
        echo "[mcp-smoke] PASS  $1"
    else
        echo "[mcp-smoke] FAIL  $1"
        local f="$OUT_DIR/probe_$1.probe"
        if [ -s "$f" ]; then
            sed -n '1,6p' "$f" | sed 's/^/[mcp-smoke]   | /'
        else
            echo "[mcp-smoke]   | (no verdict written — client failed before it could report)"
        fi
    fi
}

rc=0
if [ "$MODE" = all ]; then
    # One run, every client; report all failures.
    run_probe mcp-smoke-test
    for c in $CLIENTS; do
        if verdict_ok "$c"; then report "$c" OK; else report "$c" FAIL; rc=1; fi
    done
else
    # Fail-fast: one client at a time, stop the moment one fails.
    for c in $CLIENTS; do
        rm -f "$OUT_DIR/probe_$c.probe"
        echo "[mcp-smoke] probing $c ..."
        run_probe "probe-$c"
        if verdict_ok "$c"; then
            report "$c" OK
        else
            report "$c" FAIL
            echo "[mcp-smoke] $c failed — stopping (MODE=all to probe the rest)." >&2
            exit 1
        fi
    done
fi

if [ "$rc" -ne 0 ]; then
    echo "[mcp-smoke] one or more clients failed." >&2
else
    echo "[mcp-smoke] all clients OK."
fi
exit "$rc"
