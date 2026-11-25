#!/usr/bin/env bash
# Publish a gaviero release for the version in Cargo.toml.
#
# The release version lives in one place: [workspace.package] version in the
# root Cargo.toml. To release, bump it and run this script. It:
#
#   1. reads the version through `cargo metadata` and builds the tag v<version>;
#   2. refreshes Cargo.lock for the new version;
#   3. checks that you are on main, up to date with origin, that no file
#      other than Cargo.toml / Cargo.lock has uncommitted changes, and that
#      the tag exists neither locally nor on origin;
#   4. commits Cargo.toml + Cargo.lock as "Release v<version>" if they
#      changed, and pushes the branch;
#   5. creates the annotated tag and pushes it, which starts
#      .github/workflows/release.yml (validate, build Linux / macOS /
#      Windows, publish the GitHub release);
#   6. follows the workflow run with `gh` (when installed) and prints the
#      release URL.
#
# Nothing is pushed if a check fails. If the branch push is rejected (branch
# protection), merge the version bump through a pull request and run the
# script again from the updated main: with nothing left to commit it only
# tags and pushes the tag.
#
# Usage:
#   ./scripts/release.sh                # release the version in Cargo.toml
#   ./scripts/release.sh --dry-run      # run every check, change nothing
#   ./scripts/release.sh --no-watch     # push the tag and return without following the run
#   ./scripts/release.sh --any-branch   # release from the current branch even when it is not main
#   ./scripts/release.sh --allow-dirty  # allow uncommitted changes outside Cargo.toml / Cargo.lock
#                                       # (not part of the release: it is built from the pushed commit)
#
# Requires git, cargo and jq; gh is optional.

set -euo pipefail

dry_run=0
no_watch=0
any_branch=0
allow_dirty=0

usage() { sed -n '2,/^$/s/^# \{0,1\}//p' "$0"; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run) dry_run=1 ;;
        --no-watch) no_watch=1 ;;
        --any-branch) any_branch=1 ;;
        --allow-dirty) allow_dirty=1 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "error: unknown argument '$1'" >&2; exit 2 ;;
    esac
    shift
done

cd "$(dirname "${BASH_SOURCE[0]}")/.."

if [[ -t 1 ]]; then
    red=$'\e[31m'; cyan=$'\e[36m'; gray=$'\e[90m'; reset=$'\e[0m'
else
    red=''; cyan=''; gray=''; reset=''
fi

fail() { echo "${red}error: $*${reset}"; exit 1; }
step() { echo "${cyan}==> $*${reset}"; }

# Runs a command and stops the script when it fails.
run() {
    local code=0
    "$@" || code=$?
    [[ $code -eq 0 ]] || fail "$* failed (exit $code)"
}

# Runs a mutating command, or only prints it with --dry-run.
change() {
    if [[ $dry_run -eq 1 ]]; then
        echo "${gray}    [dry run] $*${reset}"
    else
        run "$@"
    fi
}

for tool in git cargo jq; do
    command -v "$tool" >/dev/null || fail "$tool is not on PATH"
done

# --- 1. Version -------------------------------------------------------------
metadata_json="$(cargo metadata --no-deps --format-version 1)" || fail 'cargo metadata failed'
mapfile -t versions < <(jq -r '.packages[]
    | select(.name == "gaviero-tui" or .name == "gaviero-cli" or .name == "gaviero-mcp-shim")
    | .version' <<<"$metadata_json" | sort -u)
if [[ ${#versions[@]} -ne 1 ]]; then
    fail "the shipped crates disagree on their version ($(IFS=,; echo "${versions[*]}")); they should all use version.workspace = true"
fi
version="${versions[0]}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || fail "version '$version' is not MAJOR.MINOR.PATCH[-PRE]"
tag="v$version"
pre=''
[[ "$version" == *-* ]] && pre=' (pre-release)'
step "Releasing $tag$pre"

# --- 2. Cargo.lock ----------------------------------------------------------
if ! cargo metadata --locked --format-version 1 >/dev/null 2>&1; then
    step 'Cargo.lock is out of date for this version; refreshing it'
    change cargo update --workspace
fi

# --- 3. Checks --------------------------------------------------------------
branch="$(git rev-parse --abbrev-ref HEAD)"
[[ "$branch" != HEAD ]] || fail 'detached HEAD; check out a branch first'
if [[ "$branch" != main && $any_branch -eq 0 ]]; then
    fail "on branch '$branch', not main (pass --any-branch to release from it anyway)"
fi

bump_files=()
other_dirty=()
while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    path="${line:3}"
    case "$path" in
        Cargo.toml|Cargo.lock) bump_files+=("$path") ;;
        *) other_dirty+=("$path") ;;
    esac
done < <(git status --porcelain --untracked-files=no)
if [[ ${#other_dirty[@]} -gt 0 && $allow_dirty -eq 0 ]]; then
    fail "uncommitted changes outside Cargo.toml / Cargo.lock (commit or stash them, or pass --allow-dirty):
    $(printf '%s\n    ' "${other_dirty[@]}" | sed '$d')"
fi
if [[ ${#bump_files[@]} -gt 0 ]]; then
    # Only the version line may have changed in Cargo.toml: anything else
    # belongs in its own commit, not in "Release vX".
    if git diff --unified=0 HEAD -- Cargo.toml | grep -E '^[+-][^+-]' | grep -qvE '^[+-]version = "'; then
        fail 'Cargo.toml has uncommitted changes besides the version line; commit them separately first'
    fi
fi

step 'Fetching origin'
run git fetch --quiet origin
upstream="origin/$branch"
has_upstream=0
git rev-parse --verify --quiet "$upstream" >/dev/null 2>&1 && has_upstream=1
if [[ $has_upstream -eq 1 ]]; then
    behind="$(git rev-list --count "HEAD..$upstream")"
    [[ "$behind" -eq 0 ]] || fail "$branch is $behind commit(s) behind $upstream; pull first"
fi

if git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null 2>&1; then
    fail "tag $tag already exists locally; bump [workspace.package] version in Cargo.toml"
fi
if git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
    fail "tag $tag already exists on origin; bump [workspace.package] version in Cargo.toml"
fi

# --- 4. Commit and push the version bump ------------------------------------
if [[ ${#bump_files[@]} -gt 0 ]]; then
    step "Committing the version bump ($(IFS=,; echo "${bump_files[*]}" | sed 's/,/, /g'))"
    change git commit --quiet -m "Release $tag" -- Cargo.toml Cargo.lock
fi
ahead=1
if [[ $has_upstream -eq 1 && $dry_run -eq 0 ]]; then
    ahead="$(git rev-list --count "$upstream..HEAD")"
fi
if [[ "$ahead" -gt 0 ]]; then
    step "Pushing $branch"
    change git push origin "$branch"
fi

# --- 5. Tag -----------------------------------------------------------------
step "Tagging $(git rev-parse --short HEAD) as $tag and pushing the tag"
change git tag -a "$tag" -m "gaviero $tag"
change git push origin "refs/tags/$tag"

if [[ $dry_run -eq 1 ]]; then
    step 'Dry run complete: nothing was committed, tagged or pushed'
    exit 0
fi

# --- 6. Follow the workflow -------------------------------------------------
repo_url="$(git remote get-url origin)"
repo_url="${repo_url%.git}"
if [[ $no_watch -eq 1 ]] || ! command -v gh >/dev/null; then
    echo "The release workflow is running: $repo_url/actions/workflows/release.yml"
    echo "The release will appear at:      $repo_url/releases/tag/$tag"
    exit 0
fi

step 'Waiting for the release workflow to start'
run_id=''
for _ in $(seq 1 20); do
    sleep 3
    run_id="$(gh run list --workflow release.yml --event push --branch "$tag" --limit 1 \
        --json databaseId --jq '.[0].databaseId' 2>/dev/null || true)"
    [[ -n "$run_id" ]] && break
done
if [[ -z "$run_id" ]]; then
    echo "Could not find the workflow run yet; check $repo_url/actions/workflows/release.yml"
    exit 0
fi
gh run watch "$run_id" --exit-status || fail "the release workflow failed: $repo_url/actions/runs/$run_id"
release_url="$(gh release view "$tag" --json url --jq '.url')"
step "Released: $release_url"
