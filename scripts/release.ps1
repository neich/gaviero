<#
.SYNOPSIS
  Publish a gaviero release for the version in Cargo.toml.

.DESCRIPTION
  The release version lives in one place: [workspace.package] version in the
  root Cargo.toml. To release, bump it and run this script. It:

    1. reads the version through `cargo metadata` and builds the tag v<version>;
    2. refreshes Cargo.lock for the new version;
    3. checks that you are on main, up to date with origin, that no file
       other than Cargo.toml / Cargo.lock has uncommitted changes, and that
       the tag exists neither locally nor on origin;
    4. commits Cargo.toml + Cargo.lock as "Release v<version>" if they
       changed, and pushes the branch;
    5. creates the annotated tag and pushes it, which starts
       .github/workflows/release.yml (validate, build Linux / macOS /
       Windows, publish the GitHub release);
    6. follows the workflow run with `gh` (when installed) and prints the
       release URL.

  Nothing is pushed if a check fails. If the branch push is rejected (branch
  protection), merge the version bump through a pull request and run the
  script again from the updated main: with nothing left to commit it only
  tags and pushes the tag.

.EXAMPLE
  ./scripts/release.ps1             # release the version in Cargo.toml
  ./scripts/release.ps1 -DryRun     # run every check, change nothing
  ./scripts/release.ps1 -NoWatch    # push the tag and return without following the run
#>
#Requires -Version 7
param(
    [switch]$DryRun,
    [switch]$NoWatch,
    # Release from the current branch even when it is not main.
    [switch]$AnyBranch,
    # Allow uncommitted changes to files other than Cargo.toml / Cargo.lock.
    # They are not part of the release: it is built from the pushed commit.
    [switch]$AllowDirty
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

function Fail([string]$message) {
    Write-Host "error: $message" -ForegroundColor Red
    exit 1
}

function Step([string]$message) {
    Write-Host "==> $message" -ForegroundColor Cyan
}

# Runs a native command and stops the script when it fails.
function Run([string]$exe, [string[]]$arguments) {
    & $exe @arguments
    if ($LASTEXITCODE -ne 0) { Fail "$exe $($arguments -join ' ') failed (exit $LASTEXITCODE)" }
}

# Runs a mutating command, or only prints it with -DryRun.
function Change([string]$exe, [string[]]$arguments) {
    if ($DryRun) {
        Write-Host "    [dry run] $exe $($arguments -join ' ')" -ForegroundColor DarkGray
    } else {
        Run $exe $arguments
    }
}

foreach ($tool in 'git', 'cargo') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { Fail "$tool is not on PATH" }
}

# --- 1. Version -------------------------------------------------------------
$metadataJson = cargo metadata --no-deps --format-version 1
if ($LASTEXITCODE -ne 0) { Fail 'cargo metadata failed' }
$packages = ($metadataJson | ConvertFrom-Json).packages
$shipped = 'gaviero-tui', 'gaviero-cli', 'gaviero-mcp-shim'
$versions = $shipped | ForEach-Object {
    $name = $_
    ($packages | Where-Object { $_.name -eq $name }).version
} | Sort-Object -Unique
if (@($versions).Count -ne 1) {
    Fail "the shipped crates disagree on their version ($($versions -join ', ')); they should all use version.workspace = true"
}
$version = [string]$versions
if ($version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') { Fail "version '$version' is not MAJOR.MINOR.PATCH[-PRE]" }
$tag = "v$version"
Step "Releasing $tag$(if ($version.Contains('-')) { ' (pre-release)' })"

# --- 2. Cargo.lock ----------------------------------------------------------
cargo metadata --locked --format-version 1 *> $null
if ($LASTEXITCODE -ne 0) {
    Step 'Cargo.lock is out of date for this version; refreshing it'
    Change 'cargo' @('update', '--workspace')
}

# --- 3. Checks --------------------------------------------------------------
$branch = (git rev-parse --abbrev-ref HEAD).Trim()
if ($branch -eq 'HEAD') { Fail 'detached HEAD; check out a branch first' }
if ($branch -ne 'main' -and -not $AnyBranch) { Fail "on branch '$branch', not main (pass -AnyBranch to release from it anyway)" }

$dirty = @(git status --porcelain --untracked-files=no | ForEach-Object { $_.Substring(3) })
$bumpFiles = @($dirty | Where-Object { $_ -in 'Cargo.toml', 'Cargo.lock' })
$otherDirty = @($dirty | Where-Object { $_ -notin 'Cargo.toml', 'Cargo.lock' })
if ($otherDirty.Count -gt 0 -and -not $AllowDirty) {
    Fail "uncommitted changes outside Cargo.toml / Cargo.lock (commit or stash them, or pass -AllowDirty):`n    $($otherDirty -join "`n    ")"
}
if ($bumpFiles.Count -gt 0) {
    # Only the version line may have changed in Cargo.toml: anything else
    # belongs in its own commit, not in "Release vX".
    $tomlChanges = @(git diff --unified=0 HEAD -- Cargo.toml |
        Where-Object { $_ -match '^[+-][^+-]' })
    if ($tomlChanges | Where-Object { $_ -notmatch '^[+-]version = "' }) {
        Fail "Cargo.toml has uncommitted changes besides the version line; commit them separately first"
    }
}

Step 'Fetching origin'
Run 'git' @('fetch', '--quiet', 'origin')
$upstream = "origin/$branch"
git rev-parse --verify --quiet $upstream *> $null
if ($LASTEXITCODE -eq 0) {
    $behind = [int](git rev-list --count "HEAD..$upstream")
    if ($behind -gt 0) { Fail "$branch is $behind commit(s) behind $upstream; pull first" }
}

git rev-parse --verify --quiet "refs/tags/$tag" *> $null
if ($LASTEXITCODE -eq 0) { Fail "tag $tag already exists locally; bump [workspace.package] version in Cargo.toml" }
git ls-remote --exit-code --tags origin "refs/tags/$tag" *> $null
if ($LASTEXITCODE -eq 0) { Fail "tag $tag already exists on origin; bump [workspace.package] version in Cargo.toml" }

# --- 4. Commit and push the version bump ------------------------------------
if ($bumpFiles.Count -gt 0) {
    Step "Committing the version bump ($($bumpFiles -join ', '))"
    Change 'git' @('commit', '--quiet', '-m', "Release $tag", '--', 'Cargo.toml', 'Cargo.lock')
}
$ahead = 1
git rev-parse --verify --quiet $upstream *> $null
if ($LASTEXITCODE -eq 0 -and -not $DryRun) { $ahead = [int](git rev-list --count "$upstream..HEAD") }
if ($ahead -gt 0) {
    Step "Pushing $branch"
    Change 'git' @('push', 'origin', $branch)
}

# --- 5. Tag -----------------------------------------------------------------
Step "Tagging $(git rev-parse --short HEAD) as $tag and pushing the tag"
Change 'git' @('tag', '-a', $tag, '-m', "gaviero $tag")
Change 'git' @('push', 'origin', "refs/tags/$tag")

if ($DryRun) {
    Step 'Dry run complete: nothing was committed, tagged or pushed'
    exit 0
}

# --- 6. Follow the workflow -------------------------------------------------
$repoUrl = (git remote get-url origin) -replace '\.git$', ''
if ($NoWatch -or -not (Get-Command gh -ErrorAction SilentlyContinue)) {
    Write-Host "The release workflow is running: $repoUrl/actions/workflows/release.yml"
    Write-Host "The release will appear at:      $repoUrl/releases/tag/$tag"
    exit 0
}

Step 'Waiting for the release workflow to start'
$runId = $null
for ($i = 0; $i -lt 20 -and -not $runId; $i++) {
    Start-Sleep -Seconds 3
    $runId = gh run list --workflow release.yml --event push --branch $tag --limit 1 --json databaseId --jq '.[0].databaseId' 2>$null
}
if (-not $runId) {
    Write-Host "Could not find the workflow run yet; check $repoUrl/actions/workflows/release.yml"
    exit 0
}
gh run watch $runId --exit-status
if ($LASTEXITCODE -ne 0) { Fail "the release workflow failed: $repoUrl/actions/runs/$runId" }
$releaseUrl = gh release view $tag --json url --jq '.url'
Step "Released: $releaseUrl"
