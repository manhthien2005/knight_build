#Requires -Version 7.0
<#
.SYNOPSIS
Fail-closed contract for the elevated foreign-owner ACL runner.

.DESCRIPTION
Proves the elevated runner refuses to run without explicit consent, never self-elevates, and requires the
caller to already be elevated instead of prompting for consent.

This test changes no privilege and launches no game.
#>
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = Split-Path -Parent $PSScriptRoot
$runner = Join-Path $projectRoot 'scripts\Invoke-WindowsAccountUiElevatedAclTest.ps1'

if (-not (Test-Path -LiteralPath $runner -PathType Leaf)) {
    throw "RED: elevated runner is missing: $runner"
}

function Assert-True {
    param(
        [Parameter(Mandatory)] [bool] $Condition,
        [Parameter(Mandatory)] [string] $Because
    )

    if (-not $Condition) {
        throw $Because
    }
}

function Assert-Equal {
    param(
        [Parameter(Mandatory)] $Actual,
        [Parameter(Mandatory)] $Expected,
        [Parameter(Mandatory)] [string] $Because
    )

    if ($Actual -ne $Expected) {
        throw "$Because. Expected '$Expected', got '$Actual'."
    }
}

# --- Static contract ---------------------------------------------------------------------------
$source = Get-Content -LiteralPath $runner -Raw
$ast = [System.Management.Automation.Language.Parser]::ParseInput($source, [ref]$null, [ref]$null)

$parameters = $ast.ParamBlock.Parameters | ForEach-Object { $_.Name.VariablePath.UserPath }
Assert-True ($parameters -contains 'AllowElevation') `
    'The elevated runner must declare a top-level -AllowElevation switch.'
# Elevation and game launch stay separate decisions.
Assert-True ($parameters -notcontains 'AllowGameLaunch') `
    'The elevated runner must not accept -AllowGameLaunch.'

Assert-True ($source -match '(?m)^\s*if \(-not \$AllowElevation\) \{') `
    'The elevated runner must refuse to run when consent is absent.'
Assert-True ($source -match 'WindowsBuiltInRole\]::Administrator') `
    'The elevated runner must require the caller to already be elevated.'
Assert-True ($source -match '--ignored') 'The elevated runner must target ignored tests explicitly.'
Assert-True ($source -match '--exact') 'The elevated runner must name its test exactly.'

# The whole point of this contract: it must never escalate on the operator's behalf.
foreach ($forbidden in @('-Verb\s+RunAs', 'runas', 'Start-Process\s+.*powershell')) {
    Assert-True (-not ($source -match $forbidden)) `
        "The elevated runner must never self-elevate (matched '$forbidden')."
}
Assert-True ($source -match 'never elevates itself') `
    'The elevated runner must state that it never elevates itself.'

# --- Behavioral contract -----------------------------------------------------------------------
$output = & (Join-Path $PSHOME 'pwsh.exe') -NoProfile -File $runner 2>&1
$exitCode = $LASTEXITCODE
Assert-Equal $exitCode 2 'The elevated runner must exit 2 without -AllowElevation.'
Assert-True ("$output" -match 'Refusing to run') 'The refusal must state why it refused.'

# With consent but without an elevated session it must still refuse, using a distinct code.
$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [System.Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)) {
    $consented = & (Join-Path $PSHOME 'pwsh.exe') -NoProfile -File $runner -AllowElevation 2>&1
    $consentedExit = $LASTEXITCODE
    Assert-Equal $consentedExit 3 `
        'With consent but no elevation the runner must exit 3 rather than prompting.'
    Assert-True ("$consented" -match 'elevated PowerShell session') `
        'The runner must tell the operator to start an elevated session themselves.'
}

Write-Output 'PASS: Windows account UI elevated ACL runner contract'
