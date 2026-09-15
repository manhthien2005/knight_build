#Requires -Version 7.0
<#
.SYNOPSIS
Runs the elevated foreign-owner ACL repair evidence tier.

.DESCRIPTION
Portable root repair can take ownership of a foreign-owned entry only when the caller already holds the
required privilege. That case cannot be proven unelevated, so it lives here as an ignored test.

This runner is fail-closed twice over: without -AllowElevation it refuses to run, and it never elevates
itself. If the caller is not already elevated it exits nonzero with guidance instead of prompting for
consent, so no script can silently escalate.
#>
[CmdletBinding()]
param(
    # Explicit operator consent to run the elevated evidence tier.
    [switch] $AllowElevation
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $AllowElevation) {
    # Written straight to stderr: Write-Error would terminate before the exit code is set.
    $Host.UI.WriteErrorLine('Refusing to run: pass -AllowElevation to run the elevated ACL evidence tier.')
    exit 2
}

$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [System.Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)) {
    # Deliberately no self-elevation: the operator must start an elevated shell themselves.
    $Host.UI.WriteErrorLine('Refusing to run: start an elevated PowerShell session first. This runner never elevates itself.')
    exit 3
}

$projectRoot = Split-Path -Parent $PSScriptRoot
$ignoredTest = 'portable_open_repairs_a_foreign_owned_entry_when_elevated'

& (Join-Path $PSScriptRoot 'Invoke-Cargo.ps1') `
    test --locked --manifest-path (Join-Path $projectRoot 'Cargo.toml') `
    -p zeus-core --test store_bootstrap $ignoredTest -- --exact --ignored --nocapture --test-threads=1
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

Write-Output 'PASS: Windows account UI elevated ACL evidence'
