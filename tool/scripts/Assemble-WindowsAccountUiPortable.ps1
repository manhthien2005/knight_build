#Requires -Version 7.0
<#
.SYNOPSIS
Assembles the portable Zeus account manager folder.

.DESCRIPTION
Copies the locked release binary and, when supplied, a locally provisioned runtime into a fresh output
root, then emits a bounded inventory. The script installs nothing: it never writes to the registry,
never mutates PATH, and never creates a shortcut or uninstaller.

The output root must not already exist, so an accidental run can never overwrite an operator's live
data directory.
#>
[CmdletBinding()]
param(
    # Fresh directory to create. An existing path is rejected.
    [Parameter(Mandatory)]
    [string] $OutputRoot,

    # Optional locally provisioned runtime root to copy under runtimes/windows-x64/<pinned>.
    [string] $RuntimeSource,

    # Game JAR the assembled runtime should launch, as a name inside the runtime's game/ directory.
    # Only the copied descriptor is rewritten; $RuntimeSource is never modified. Leave unset to keep
    # whatever JAR the source descriptor already pins.
    [string] $GameJar,

    # Skip the release build and reuse the existing binary.
    [switch] $SkipBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = Split-Path -Parent $PSScriptRoot
$pinnedRuntimeDirectory = 'temurin-11.0.32+9_microemu-2.0.4_ko402'

if (Test-Path -LiteralPath $OutputRoot) {
    throw "OutputRoot must be a fresh path that does not exist yet: $OutputRoot"
}

if ($GameJar) {
    if (-not $RuntimeSource) {
        throw 'GameJar requires RuntimeSource: there is no copied descriptor to repoint otherwise.'
    }
    if ($GameJar -ne (Split-Path -Leaf $GameJar)) {
        throw "GameJar must be a plain file name inside the runtime's game directory: $GameJar"
    }
}

if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot 'Invoke-Cargo.ps1') build --locked --workspace --release
    if ($LASTEXITCODE -ne 0) {
        throw 'Release build failed.'
    }
}

$releaseBinary = Join-Path $projectRoot 'target\release\zeus-ui.exe'
if (-not (Test-Path -LiteralPath $releaseBinary -PathType Leaf)) {
    throw "Release binary is missing: $releaseBinary"
}

# Create the guarded child layout. Nothing outside OutputRoot is touched.
$null = New-Item -ItemType Directory -Path $OutputRoot -Force
$runtimeParent = Join-Path $OutputRoot 'runtimes\windows-x64'
$null = New-Item -ItemType Directory -Path $runtimeParent -Force
# `data/` is deliberately NOT pre-created. The application creates it with a protected owner-only DACL;
# an empty directory created here would carry inherited ACLs and fail the private-root check.

Copy-Item -LiteralPath $releaseBinary -Destination (Join-Path $OutputRoot 'zeus-ui.exe')

$runtimeRoot = Join-Path $runtimeParent $pinnedRuntimeDirectory
if ($RuntimeSource) {
    if (-not (Test-Path -LiteralPath $RuntimeSource -PathType Container)) {
        throw "RuntimeSource is not a directory: $RuntimeSource"
    }
    Copy-Item -LiteralPath $RuntimeSource -Destination $runtimeRoot -Recurse
    # A plain copy inherits the destination's ACLs, and the application requires the pinned runtime to
    # be reachable only by the current user and SYSTEM. Without this the copied runtime is rejected as
    # insecure and the tool boots into `Tool chưa sẵn sàng`.
    Import-Module (Join-Path $PSScriptRoot 'Zeus.RuntimeProvisioning.psm1') -Force
    Protect-ZeusRuntimeCopy -Path $runtimeRoot
}
else {
    # The runtime is provisioned separately; the empty parent keeps the layout explicit.
    $null = New-Item -ItemType Directory -Path $runtimeRoot -Force
}

if ($GameJar) {
    # The application reads exactly one descriptor name, so pointing it at a different JAR means
    # rewriting the copied descriptor. Size and digest are measured from the JAR itself rather than
    # supplied, because a descriptor that disagrees with its payload is rejected at boot.
    $descriptorPath = Join-Path $runtimeRoot 'runtime-descriptor.json'
    if (-not (Test-Path -LiteralPath $descriptorPath -PathType Leaf)) {
        throw "Copied runtime has no descriptor to repoint: $descriptorPath"
    }
    $jarPath = Join-Path $runtimeRoot "game\$GameJar"
    if (-not (Test-Path -LiteralPath $jarPath -PathType Leaf)) {
        throw "GameJar is not present in the copied runtime: $jarPath"
    }
    $descriptor = Get-Content -LiteralPath $descriptorPath -Raw | ConvertFrom-Json
    $descriptor.game.jar = "game/$GameJar"
    $descriptor.game.jar_size = (Get-Item -LiteralPath $jarPath).Length
    $descriptor.game.jar_sha256 =
        (Get-FileHash -LiteralPath $jarPath -Algorithm SHA256).Hash.ToLowerInvariant()
    # A distinct id keeps a modded runtime from being mistaken for the pinned vanilla one in a data
    # root that has already registered the latter.
    $jarStem = [System.IO.Path]::GetFileNameWithoutExtension($GameJar).ToLowerInvariant()
    $descriptor.runtime_id = "$($descriptor.runtime_id)_$jarStem"
    $descriptor | ConvertTo-Json -Depth 12 |
        Set-Content -LiteralPath $descriptorPath -Encoding utf8NoBOM
}

# Bounded inventory. Paths are reported relative to the output root so no absolute path leaks.
$inventory = Get-ChildItem -LiteralPath $OutputRoot -Recurse -File |
    ForEach-Object { $_.FullName.Substring($OutputRoot.Length).TrimStart('\', '/') } |
    Sort-Object

[pscustomobject]@{
    Root         = $OutputRoot
    Executable   = 'zeus-ui.exe'
    DataRoot     = 'data'
    RuntimeRoot  = "runtimes\windows-x64\$pinnedRuntimeDirectory"
    GameJar      = if ($GameJar) { "game/$GameJar" } else { '(source descriptor)' }
    FileCount    = @($inventory).Count
    Files        = $inventory
}
