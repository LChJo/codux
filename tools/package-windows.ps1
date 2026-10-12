param(
    [string]$Target = "x86_64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $RepoRoot

if ($env:GITHUB_ACTIONS -eq "true") {
    throw "tools/package-windows.ps1 is for local packages. CI releases must keep using the tag-derived version."
}

Write-Host "Incrementing the local package version..."
$versionLines = & node "apps/desktop/scripts/release/bump-local-version.mjs"
if ($LASTEXITCODE -ne 0) {
    throw "Version bump failed with exit code $LASTEXITCODE"
}
$versionLine = $versionLines | Where-Object { $_ -match '^version=' } | Select-Object -Last 1
if (-not $versionLine) {
    throw "Version bump did not report the new version."
}
$packageVersion = $versionLine.Substring("version=".Length).Trim()
Write-Host "Building Codux $packageVersion..."

& (Join-Path $PSScriptRoot "build-windows.ps1") -Target $Target
if ($LASTEXITCODE -ne 0) {
    throw "Windows build failed with exit code $LASTEXITCODE"
}

$builtExecutable = Join-Path $RepoRoot "target\$Target\release\codux.exe"
# `codux.exe` is a GUI-subsystem binary. Under Windows PowerShell 5.1, capturing
# its stdout with `& $exe --version` intermittently yields nothing, which made
# this check fail on a binary that had just printed its version correctly during
# the build's own smoke test. Redirecting through Start-Process is reliable.
$versionOutputFile = Join-Path ([System.IO.Path]::GetTempPath()) "codux-package-version.txt"
Remove-Item $versionOutputFile -Force -ErrorAction SilentlyContinue
$versionProcess = Start-Process -FilePath $builtExecutable `
    -ArgumentList "--version" `
    -NoNewWindow -Wait -PassThru `
    -RedirectStandardOutput $versionOutputFile
$reportedVersion = (Get-Content $versionOutputFile -Raw -ErrorAction SilentlyContinue)
if ($null -ne $reportedVersion) { $reportedVersion = $reportedVersion.Trim() }
Remove-Item $versionOutputFile -Force -ErrorAction SilentlyContinue
if ($versionProcess.ExitCode -ne 0 -or $reportedVersion -ne "codux $packageVersion") {
    throw "Built binary version mismatch: expected 'codux $packageVersion', got '$reportedVersion' (exit code $($versionProcess.ExitCode))."
}

$previousTarget = $env:CARGO_BUILD_TARGET
$previousBuildId = $env:RELEASE_BUILD_ID
$previousWriteSha256 = $env:RELEASE_WRITE_SHA256
$previousPreparedVersion = $env:CODUX_PACKAGE_VERSION_PREPARED
try {
    $env:CARGO_BUILD_TARGET = $Target
    $env:RELEASE_BUILD_ID = "windows-x86_64"
    $env:RELEASE_WRITE_SHA256 = "true"
    $env:CODUX_PACKAGE_VERSION_PREPARED = $packageVersion
    & node "apps/desktop/scripts/release/package-gpui.mjs"
    if ($LASTEXITCODE -ne 0) {
        throw "Windows packaging failed with exit code $LASTEXITCODE"
    }
}
finally {
    $env:CARGO_BUILD_TARGET = $previousTarget
    $env:RELEASE_BUILD_ID = $previousBuildId
    $env:RELEASE_WRITE_SHA256 = $previousWriteSha256
    $env:CODUX_PACKAGE_VERSION_PREPARED = $previousPreparedVersion
}

Write-Host "Created Codux $packageVersion installers in release-artifacts\windows-x86_64."
