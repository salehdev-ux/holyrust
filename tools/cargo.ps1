#!/usr/bin/env pwsh
# Cargo wrapper for this host.
#
# Why it exists (docs/PROGRESS.md TOS-P2.3): this machine's wall clock jumps
# BACKWARDS, so build outputs can end up timestamped in the future (measured:
# artefacts at 17:06 while the clock read 16:31). Cargo then judges every source
# older than its own outputs, reports "Fresh", and silently links the previous
# objects — three different implementations produced byte-identical binaries
# before this was found. Touching the sources cannot fix it; the future-dated
# target has to go.
#
# Usage:  powershell -NoProfile -File tools/cargo.ps1 test --release
$ErrorActionPreference = 'Stop'

$repo = Split-Path $PSScriptRoot -Parent
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not (Test-Path $cargo)) { $cargo = 'cargo' }

$now = Get-Date
foreach ($t in @((Join-Path $repo 'target'), (Join-Path $repo 'kernel\target'))) {
    if (-not (Test-Path $t)) { continue }
    $future = Get-ChildItem -LiteralPath $t -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -gt $now.AddMinutes(1) } | Select-Object -First 1
    if ($future) {
        Write-Host "cargo.ps1: future-dated artefact ($($future.Name)); clearing $t for a real rebuild"
        Remove-Item -LiteralPath $t -Recurse -Force -ErrorAction SilentlyContinue
    }
}

& $cargo @args
exit $LASTEXITCODE
