#Requires -Version 5.1
<#
.SYNOPSIS
    Build, lint, test and package usbscan.

.DESCRIPTION
    Runs the same checks as CI, builds the release executable and writes
    distributable files to .\dist.

.PARAMETER Configuration
    Cargo profile to build. Default: release.

.PARAMETER SkipLint
    Skip `cargo fmt --check` and `cargo clippy`.

.PARAMETER SkipTests
    Skip `cargo test --workspace`.

.PARAMETER Clean
    Run `cargo clean` before building.

.PARAMETER NoPackage
    Build only; do not copy files to .\dist.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\build.ps1

.EXAMPLE
    .\build.ps1 -SkipTests -Clean
#>
[CmdletBinding()]
param(
    [ValidateSet('release', 'debug')]
    [string]$Configuration = 'release',

    [switch]$SkipLint,
    [switch]$SkipTests,
    [switch]$Clean,
    [switch]$NoPackage
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Root = $PSScriptRoot
if (-not $Root) {
    $Root = Split-Path -Parent $MyInvocation.MyCommand.Path
}
Set-Location -LiteralPath $Root

function Invoke-Cargo {
    param([string[]]$CargoArguments)

    Write-Host "> cargo $($CargoArguments -join ' ')" -ForegroundColor Cyan
    & cargo @CargoArguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($CargoArguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'cargo was not found. Install Rust from https://rustup.rs/ and reopen the terminal.'
}
if (-not (Get-Command rustc -ErrorAction SilentlyContinue)) {
    throw 'rustc was not found. The Rust toolchain is incomplete.'
}

$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
$rustcVersion = (& rustc --version) -join ' '
$hostLine = (& rustc -vV) | Select-String -Pattern '^host:'
$hostTriple = if ($hostLine) { ($hostLine.Line -replace '^host:\s*', '') } else { '' }
if (-not $hostTriple) {
    throw 'Unable to detect the Rust host target.'
}

$arch = switch -Regex ($hostTriple) {
    '^x86_64' { 'x64'; break }
    '^aarch64' { 'arm64'; break }
    '^i686' { 'x86'; break }
    default { 'unknown' }
}

$cargoToml = Get-Content -LiteralPath (Join-Path $Root 'Cargo.toml') -Raw
$versionMatch = [regex]::Match($cargoToml, '(?m)^\s*version\s*=\s*"([^"]+)"')
if (-not $versionMatch.Success) {
    throw 'Unable to read the workspace version from Cargo.toml.'
}
$version = $versionMatch.Groups[1].Value

Write-Host ''
Write-Host 'usbscan build' -ForegroundColor Green
Write-Host "  version : $version"
Write-Host "  profile : $Configuration"
Write-Host "  host    : $hostTriple"
Write-Host "  rustc   : $rustcVersion"
Write-Host "  root    : $Root"
Write-Host ''

if ($Clean) {
    Invoke-Cargo -CargoArguments @('clean')
}

if (-not $SkipLint) {
    Invoke-Cargo -CargoArguments @('fmt', '--all', '--', '--check')
    Invoke-Cargo -CargoArguments @('clippy', '--workspace', '--all-targets', '--', '-D', 'warnings')
}

if (-not $SkipTests) {
    Invoke-Cargo -CargoArguments @('test', '--workspace')
}

$buildArguments = @('build', '-p', 'usbscan-app')
if ($Configuration -eq 'release') {
    $buildArguments += '--release'
}
Invoke-Cargo -CargoArguments $buildArguments

$builtExe = Join-Path $Root (Join-Path "target\$Configuration" 'usbscan.exe')
if (-not (Test-Path -LiteralPath $builtExe)) {
    throw "Build succeeded but the executable was not found: $builtExe"
}

if ($NoPackage -or $Configuration -ne 'release') {
    Write-Host ''
    Write-Host "Executable: $builtExe" -ForegroundColor Green
} else {
    $dist = Join-Path $Root 'dist'
    New-Item -ItemType Directory -Path $dist -Force | Out-Null

    $distExe = Join-Path $dist 'usbscan.exe'
    Copy-Item -LiteralPath $builtExe -Destination $distExe -Force
    foreach ($fileName in @(
        'LICENSE',
        'README.md',
        'THIRD_PARTY.md',
        'assets\icons\TABLER-ICONS-LICENSE.txt'
    )) {
        $source = Join-Path $Root $fileName
        if (Test-Path -LiteralPath $source) {
            Copy-Item -LiteralPath $source -Destination $dist -Force
        }
    }

    $archive = Join-Path $dist "usbscan-$version-win-$arch.zip"
    $archiveItems = @(
        $distExe,
        (Join-Path $dist 'LICENSE'),
        (Join-Path $dist 'README.md'),
        (Join-Path $dist 'THIRD_PARTY.md'),
        (Join-Path $dist 'TABLER-ICONS-LICENSE.txt')
    ) | Where-Object { Test-Path -LiteralPath $_ }
    Compress-Archive -LiteralPath $archiveItems -DestinationPath $archive -Force

    $checksums = foreach ($file in @($distExe, $archive)) {
        $hash = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $(Split-Path -Leaf $file)"
    }
    $checksumFile = Join-Path $dist 'SHA256SUMS.txt'
    Set-Content -LiteralPath $checksumFile -Value $checksums -Encoding ASCII

    Write-Host ''
    Write-Host 'Package ready:' -ForegroundColor Green
    Write-Host "  exe     : $distExe"
    Write-Host "  archive : $archive"
    Write-Host "  sums    : $checksumFile"
    Write-Host ''
    Get-Content -LiteralPath $checksumFile | ForEach-Object { Write-Host "  $_" }
}

$stopwatch.Stop()
Write-Host ''
Write-Host ("Done in {0:n1}s" -f $stopwatch.Elapsed.TotalSeconds) -ForegroundColor Green
