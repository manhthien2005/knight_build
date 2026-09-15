#Requires -Version 7.0
<#
.SYNOPSIS
Runs the exact-game account login evidence tier.

.DESCRIPTION
This runner launches the real pinned game and injects the fixed login script. It is fail-closed: without
an explicit -AllowGameLaunch it refuses to run and exits nonzero, so no default, CI, or source-tier
invocation can ever start the game or send input.

The runner never elevates. Elevated evidence lives in Invoke-WindowsAccountUiElevatedAclTest.ps1.
#>
[CmdletBinding()]
param(
    # Explicit operator consent to launch the real game and inject input.
    [switch] $AllowGameLaunch,

    # Directory holding the provisioned pinned runtime.
    [string] $RuntimeRoot
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $AllowGameLaunch) {
    # Written straight to stderr: Write-Error would terminate before the exit code is set.
    $Host.UI.WriteErrorLine('Refusing to run: pass -AllowGameLaunch to launch the real game and inject input.')
    exit 2
}

$projectRoot = Split-Path -Parent $PSScriptRoot
$ignoredTest = 'manager::worker::login::windows_live_runtime_tests::account_login_reaches_the_registration_prompt'

# Synthetic credentials are derived per run, so no fixed credential literal exists in the repository.
$runId = [guid]::NewGuid().ToString('N')
$syntheticUsername = "zeus_$($runId.Substring(0, 10))"
$syntheticPassword = "Pw-$($runId.Substring(10, 12))"

$previousRuntimeRoot = $env:ZEUS_EXACT_RUNTIME_ROOT
$previousUsername = $env:ZEUS_ACCOUNT_LIVE_USERNAME
$previousPassword = $env:ZEUS_ACCOUNT_LIVE_PASSWORD
try {
    if ($RuntimeRoot) {
        $env:ZEUS_EXACT_RUNTIME_ROOT = $RuntimeRoot
    }
    $env:ZEUS_ACCOUNT_LIVE_USERNAME = $syntheticUsername
    $env:ZEUS_ACCOUNT_LIVE_PASSWORD = $syntheticPassword

    # One ignored test, named exactly, run serially: concurrent input would corrupt the evidence.
    & (Join-Path $PSScriptRoot 'Invoke-Cargo.ps1') `
        test --locked --manifest-path (Join-Path $projectRoot 'Cargo.toml') `
        -p zeus-core --lib $ignoredTest -- --exact --ignored --nocapture --test-threads=1
    $exitCode = $LASTEXITCODE
}
finally {
    # The process environment is always restored, so a failed run cannot leak synthetic credentials
    # into a later command.
    $env:ZEUS_EXACT_RUNTIME_ROOT = $previousRuntimeRoot
    $env:ZEUS_ACCOUNT_LIVE_USERNAME = $previousUsername
    $env:ZEUS_ACCOUNT_LIVE_PASSWORD = $previousPassword
}

if ($exitCode -ne 0) {
    exit $exitCode
}

Write-Output 'PASS: Windows account UI live login evidence'
