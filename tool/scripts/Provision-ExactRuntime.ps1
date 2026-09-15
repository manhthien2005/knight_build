[CmdletBinding()]
param(
    [string] $RuntimeRoot = (Join-Path $PSScriptRoot '..\runtimes\windows-x64\temurin-11.0.32+9_microemu-2.0.4_ko402'),
    [string] $CacheRoot = (Join-Path $PSScriptRoot '..\.source-cache\runtime-setup'),
    [string] $GameJar,
    [string] $JreArchive,
    [string] $MicroemulatorArchive,
    [switch] $VerifyOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne
    [Runtime.InteropServices.Architecture]::X64) {
    throw 'Exact runtime provisioning requires Windows x64.'
}

Import-Module (Join-Path $PSScriptRoot 'Zeus.RuntimeProvisioning.psm1') -Force

$result = Invoke-ZeusRuntimeProvisioning -RuntimeRoot $RuntimeRoot -CacheRoot $CacheRoot `
    -GameJar $GameJar -JreArchive $JreArchive -MicroemulatorArchive $MicroemulatorArchive `
    -VerifyOnly:$VerifyOnly
$result | ConvertTo-Json -Depth 4
