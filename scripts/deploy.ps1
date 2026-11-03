<#
.SYNOPSIS
  Build the gaviero binaries in release mode and install them into ~/.local/bin.

.DESCRIPTION
  Running copies are left alone: Windows will not overwrite a running .exe but
  does allow renaming it, so each live binary is moved aside to
  <name>.exe.old-<timestamp> and the new one takes its place. Leftover .old
  files are deleted on the next deploy once nothing runs them any more.

  A gaviero TUI started through its built-in supervisor (the default) notices
  the new gaviero.exe and restarts itself in the same pane as soon as no agent,
  terminal command, unsaved file or dialog is pending. Session state (tabs,
  layout, conversations, pending turn reviews) is restored.

.EXAMPLE
  ./scripts/deploy.ps1                 # build + install; running TUIs restart when idle
  ./scripts/deploy.ps1 -NoBuild        # install the last release build only
  ./scripts/deploy.ps1 -StopRunning    # hard-kill running copies instead (loses unsaved session state)
#>
param(
    [string]$Destination = (Join-Path $HOME '.local\bin'),
    [switch]$NoBuild,
    [switch]$StopRunning
)

$ErrorActionPreference = 'Stop'
# gaviero last: a running TUI restarts once it sees its own exe change, and by
# then the CLI and the MCP shim it spawns are already the new build.
$bins = 'gaviero-cli', 'gaviero-mcp-shim', 'gaviero'
$repo = Split-Path $PSScriptRoot -Parent

Push-Location $repo
try {
    if (-not $NoBuild) {
        $binArgs = $bins | ForEach-Object { '--bin', $_ }
        cargo build --release @binArgs
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    }
    # Honour .cargo/config.toml's target-dir instead of assuming ./target.
    $targetDir = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
} finally {
    Pop-Location
}
$src = Join-Path $targetDir 'release'

foreach ($b in $bins) {
    $file = Join-Path $src "$b.exe"
    if (-not (Test-Path $file)) { throw "Missing $file (build it, or drop -NoBuild)" }
}

if ($StopRunning) {
    $running = Get-Process -Name $bins -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and (Split-Path $_.Path -Parent) -eq $Destination }
    if ($running) {
        Write-Warning ("Stopping " + (($running | ForEach-Object { "$($_.Name) (pid $($_.Id))" }) -join ', '))
        $running | Stop-Process -Force
        $running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
    }
}

New-Item -ItemType Directory -Force -Path $Destination | Out-Null

# Binaries moved aside by earlier deploys; the ones still running stay locked.
Get-ChildItem -Path $Destination -Filter '*.exe.old-*' -ErrorAction SilentlyContinue |
    ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue }

$stamp = Get-Date -Format 'yyyyMMddHHmmss'
foreach ($b in $bins) {
    $file = Join-Path $src "$b.exe"
    $dest = Join-Path $Destination "$b.exe"
    # Stage next to the target first, so the swap below is two renames on one
    # volume and the target is missing only for an instant.
    $staged = "$dest.new"
    Copy-Item -LiteralPath $file -Destination $staged -Force
    if (Test-Path -LiteralPath $dest) {
        try {
            Remove-Item -LiteralPath $dest -Force
        } catch {
            # Running: Windows allows the rename, not the delete.
            Move-Item -LiteralPath $dest -Destination "$dest.old-$stamp" -Force
        }
    }
    Move-Item -LiteralPath $staged -Destination $dest -Force
}

$live = Get-Process -Name $bins -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and (Split-Path $_.Path -Parent) -eq $Destination }
if ($live) {
    Write-Host ("Still running the previous build: " +
        (($live | Group-Object Name | ForEach-Object { "$($_.Name) x$($_.Count)" }) -join ', ') +
        ". Supervised gaviero TUIs restart once idle.")
}

Get-Item ($bins | ForEach-Object { Join-Path $Destination "$_.exe" }) |
    Format-Table Name, LastWriteTime, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 1) } } -AutoSize
