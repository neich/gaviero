#!/usr/bin/env bash
# Build the gaviero binaries in release mode and install them into ~/.local/bin.
#
# Running copies are left alone: Linux will not write into a running
# executable (ETXTBSY) but does allow replacing its directory entry, so each
# new binary is staged next to the target and renamed over it. Running
# processes keep the old inode until they exit; nothing is left to clean up.
#
# A gaviero TUI started through its built-in supervisor (the default) notices
# the new gaviero and restarts itself in the same pane as soon as no agent,
# terminal command, unsaved file or dialog is pending. Session state (tabs,
# layout, conversations, pending turn reviews) is restored.
#
# A session host (gaviero --sessions) is supervised the same way: once no tab
# has an agent, swarm or shell command running or unsaved files, and the host
# has not been typed into for 15 s, it restarts itself and every tab onto the
# new build, reopening the same tabs. Its tabs leave restarting to the host.
#
# Usage:
#   ./scripts/deploy.sh                  # build + install; running TUIs and session hosts restart when idle
#   ./scripts/deploy.sh --no-build       # install the last release build only
#   ./scripts/deploy.sh --stop-running   # hard-kill running copies instead (loses unsaved session state)
#   ./scripts/deploy.sh --destination DIR

set -euo pipefail

destination="$HOME/.local/bin"
no_build=0
stop_running=0

usage() { sed -n '2,/^$/s/^# \{0,1\}//p' "$0"; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --destination|-d) destination="$2"; shift 2 ;;
        --destination=*) destination="${1#*=}"; shift ;;
        --no-build) no_build=1; shift ;;
        --stop-running) stop_running=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "error: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

# gaviero last: a running TUI restarts once it sees its own binary change, and
# by then the CLI and the MCP shim it spawns are already the new build.
bins=(gaviero-cli gaviero-mcp-shim gaviero)
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$repo"
if [[ $no_build -eq 0 ]]; then
    bin_args=()
    for b in "${bins[@]}"; do bin_args+=(--bin "$b"); done
    cargo build --release "${bin_args[@]}"
fi
# Honour .cargo/config.toml's target-dir instead of assuming ./target.
if command -v jq >/dev/null; then
    target_dir="$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)"
else
    target_dir="$(cargo metadata --format-version 1 --no-deps |
        sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
fi
src="$target_dir/release"

for b in "${bins[@]}"; do
    [[ -f "$src/$b" ]] || { echo "error: missing $src/$b (build it, or drop --no-build)" >&2; exit 1; }
done

mkdir -p "$destination"
destination="$(cd "$destination" && pwd -P)"

# Pids of the deployed binaries running from $destination, as "pid name".
running_from_dest() {
    local pid exe
    for exe_link in /proc/[0-9]*/exe; do
        exe="$(readlink "$exe_link" 2>/dev/null)" || continue
        exe="${exe% (deleted)}"
        [[ "$(dirname "$exe")" == "$destination" ]] || continue
        for b in "${bins[@]}"; do
            if [[ "$(basename "$exe")" == "$b" ]]; then
                pid="${exe_link#/proc/}"; pid="${pid%/exe}"
                echo "$pid $b"
            fi
        done
    done
}

if [[ $stop_running -eq 1 ]]; then
    mapfile -t running < <(running_from_dest)
    if [[ ${#running[@]} -gt 0 ]]; then
        pids=()
        desc=()
        for r in "${running[@]}"; do
            pids+=("${r%% *}")
            desc+=("${r#* } (pid ${r%% *})")
        done
        echo "warning: stopping $(IFS=,; echo "${desc[*]}" | sed 's/,/, /g')" >&2
        kill -9 "${pids[@]}" 2>/dev/null || true
        for _ in $(seq 1 100); do
            alive=0
            for p in "${pids[@]}"; do kill -0 "$p" 2>/dev/null && alive=1; done
            [[ $alive -eq 0 ]] && break
            sleep 0.1
        done
    fi
fi

for b in "${bins[@]}"; do
    dest="$destination/$b"
    # Stage next to the target first, so the swap is one rename on one
    # filesystem and the target is never missing or half-written.
    staged="$dest.new"
    cp -f "$src/$b" "$staged"
    chmod 755 "$staged"
    mv -f "$staged" "$dest"
done

mapfile -t live < <(running_from_dest)
if [[ ${#live[@]} -gt 0 ]]; then
    summary="$(printf '%s\n' "${live[@]}" | cut -d' ' -f2 | sort | uniq -c |
        awk '{ printf "%s%s x%s", (NR > 1 ? ", " : ""), $2, $1 }')"
    echo "Still running the previous build: $summary. Supervised gaviero TUIs and session hosts restart once idle."
fi

printf '%-20s %-20s %6s\n' Name LastWriteTime MB
for b in "${bins[@]}"; do
    f="$destination/$b"
    printf '%-20s %-20s %6s\n' "$b" "$(date -r "$f" '+%Y-%m-%d %H:%M:%S')" \
        "$(awk -v s="$(stat -c %s "$f")" 'BEGIN { printf "%.1f", s / 1048576 }')"
done
