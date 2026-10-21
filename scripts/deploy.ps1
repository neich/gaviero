<#
.SYNOPSIS
  Build the gaviero binaries in release mode and install them into ~/.local/bin.

.EXAMPLE
  ./scripts/deploy.ps1                 # build + copy
  ./scripts/deploy.ps1 -NoBuild        # copy the last release build only
  ./scripts/deploy.ps1 -StopRunning    # kill running copies first instead of aborting
#>
param(
    [string]$Destination = (Join-Path $HOME '.local\bin'),
    [switch]$NoBuild,
    [switch]$StopRunning
)

$ErrorActionPreference = 'Stop'
$bins = 'gaviero', 'gaviero-cli', 'gaviero-mcp-shim'
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

# Windows refuses to overwrite a running .exe, so deal with live copies up front.
$running = Get-Process -Name $bins -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and (Split-Path $_.Path -Parent) -eq $Destination }
if ($running) {
    $list = ($running | ForEach-Object { "$($_.Name) (pid $($_.Id))" }) -join ', '
    if (-not $StopRunning) {
        throw "Still running from ${Destination}: $list. Close them or pass -StopRunning."
    }
    Write-Warning "Stopping $list"
    $running | Stop-Process -Force
    $running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
}

New-Item -ItemType Directory -Force -Path $Destination | Out-Null
foreach ($b in $bins) {
    $file = Join-Path $src "$b.exe"
    if (-not (Test-Path $file)) { throw "Missing $file (build it, or drop -NoBuild)" }
    Copy-Item $file $Destination -Force
}

Get-Item ($bins | ForEach-Object { Join-Path $Destination "$_.exe" }) |
    Format-Table Name, LastWriteTime, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 1) } } -AutoSize
