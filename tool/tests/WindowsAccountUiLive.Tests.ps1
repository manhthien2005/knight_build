#Requires -Version 7.0
<#
.SYNOPSIS
Fail-closed contract for the exact-game account login runner.

.DESCRIPTION
Proves the live runner cannot launch the game or inject input without explicit operator consent, that it
runs exactly one named ignored test serially, that it derives synthetic credentials per run rather than
carrying literals, and that it restores the process environment in `finally`.

This test launches no game, injects no input, and changes no privilege.
#>
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = Split-Path -Parent $PSScriptRoot
$runner = Join-Path $projectRoot 'scripts\Invoke-WindowsAccountUiLiveTest.ps1'

if (-not (Test-Path -LiteralPath $runner -PathType Leaf)) {
    throw "RED: live runner is missing: $runner"
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

# --- Static contract, read from the source ---------------------------------------------------
$source = Get-Content -LiteralPath $runner -Raw
$ast = [System.Management.Automation.Language.Parser]::ParseInput($source, [ref]$null, [ref]$null)

$parameters = $ast.ParamBlock.Parameters | ForEach-Object { $_.Name.VariablePath.UserPath }
Assert-True ($parameters -contains 'AllowGameLaunch') `
    'The live runner must declare a top-level -AllowGameLaunch switch.'
# Game launch and elevation are separate decisions; this runner must not offer elevation at all.
Assert-True ($parameters -notcontains 'AllowElevation') `
    'The live runner must not accept -AllowElevation.'

Assert-True ($source -match '(?m)^\s*if \(-not \$AllowGameLaunch\) \{') `
    'The live runner must refuse to run when consent is absent.'
Assert-True ($source -match '--ignored') 'The live runner must target ignored tests explicitly.'
Assert-True ($source -match '--exact') 'The live runner must name its test exactly.'
Assert-True ($source -match '--test-threads=1') `
    'The live runner must run serially: concurrent input would corrupt the evidence.'
Assert-True ($source -match '(?m)^\s*finally \{') `
    'The live runner must restore the process environment in finally.'
Assert-True ($source -match 'Invoke-WindowsAccountUiElevatedAclTest') `
    'The live runner must point elevated evidence at its own runner.'

# Nothing may self-elevate here.
foreach ($forbidden in @('Start-Process.*-Verb\s+RunAs', 'runas')) {
    Assert-True (-not ($source -match $forbidden)) `
        "The live runner must never self-elevate (matched '$forbidden')."
}

# Synthetic credentials are derived per run, never stored as literals.
Assert-True ($source -match '\[guid\]::NewGuid\(\)') `
    'The live runner must derive synthetic credentials per run.'
foreach ($forbidden in @('Secret-1', 'password123', 'P@ssw0rd')) {
    Assert-True (-not $source.Contains($forbidden)) `
        "The live runner must not contain the credential literal '$forbidden'."
}

# --- Behavioral contract: no consent means no run --------------------------------------------
$before = Get-Process -Name 'javaw' -ErrorAction SilentlyContinue
$output = & (Join-Path $PSHOME 'pwsh.exe') -NoProfile -File $runner 2>&1
$exitCode = $LASTEXITCODE

Assert-Equal $exitCode 2 'The live runner must exit 2 without -AllowGameLaunch.'
Assert-True ("$output" -match 'Refusing to run') 'The refusal must state why it refused.'

$after = Get-Process -Name 'javaw' -ErrorAction SilentlyContinue
Assert-Equal @($after).Count @($before).Count `
    'A refused run must not launch the game.'

Write-Output 'PASS: Windows account UI live runner contract'
