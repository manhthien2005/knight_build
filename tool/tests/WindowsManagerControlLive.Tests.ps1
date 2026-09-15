$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$runnerPath = Join-Path $PSScriptRoot '..\scripts\Invoke-WindowsManagerControlLiveTest.ps1'
$modulePath = Join-Path $PSScriptRoot '..\scripts\Zeus.TestTiers.psm1'
$sourceTierPath = Join-Path $PSScriptRoot '..\scripts\Invoke-SourceTests.ps1'

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

function Assert-ThrowsLike {
    param(
        [Parameter(Mandatory)] [scriptblock] $Action,
        [Parameter(Mandatory)] [string] $Pattern,
        [Parameter(Mandatory)] [string] $Because
    )

    try {
        & $Action
    }
    catch {
        if ($_.Exception.Message -notlike $Pattern) {
            throw "$Because. Expected error like '$Pattern', got '$($_.Exception.Message)'."
        }
        return
    }

    throw "$Because. Expected an error like '$Pattern', but the command succeeded."
}

function Get-ScriptContract {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Label
    )

    $tokens = $null
    $errors = $null
    $resolved = (Resolve-Path -LiteralPath $Path -ErrorAction Stop).ProviderPath
    $ast = [Management.Automation.Language.Parser]::ParseFile(
        $resolved,
        [ref] $tokens,
        [ref] $errors
    )
    if (@($errors).Count -ne 0) {
        throw "$Label has PowerShell parse errors."
    }
    [pscustomobject] @{
        ast = $ast
        tokens = @($tokens)
        text = [IO.File]::ReadAllText($resolved)
    }
}

if (-not (Test-Path -LiteralPath $runnerPath -PathType Leaf)) {
    throw 'RED: Windows manager control live runner is missing.'
}

Import-Module $modulePath -Force
$originalEnvironment = Get-ZeusLiveEnvironmentState
$sourceTier = Get-ScriptContract -Path $sourceTierPath -Label 'source tier'
$runner = Get-ScriptContract -Path $runnerPath -Label 'manager live runner'

Assert-Equal ([regex]::Matches(
        $sourceTier.text,
        [regex]::Escape('WindowsManagerControlLive.Tests.ps1'),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count) 1 'Source tier must invoke the manager live contract exactly once'
foreach ($forbidden in @(
        'Invoke-WindowsManagerControlLiveTest.ps1',
        'AllowGameLaunch',
        'windows-live-runtime-bridge-v1-approved',
        'temurin-11.0.32+9_microemu-2.0.4_ko402'
    )) {
    Assert-Equal $sourceTier.text.Contains($forbidden) $false `
        'Source tier must not contain manager live admission material'
}

$parameterNames = @($runner.ast.ParamBlock.Parameters | ForEach-Object {
        $_.Name.VariablePath.UserPath
    })
Assert-Equal ($parameterNames -join ',') 'RuntimeRoot,AllowGameLaunch' `
    'Manager live runner must expose only the runtime root and admission switch'
$switchParameters = @($runner.ast.ParamBlock.Parameters | Where-Object {
        $_.StaticType.FullName -eq 'System.Management.Automation.SwitchParameter'
    })
Assert-Equal $switchParameters.Count 1 'Manager live runner must expose one switch'
if ($switchParameters[0].Name.VariablePath.UserPath -cne 'AllowGameLaunch') {
    throw 'Manager live admission switch must remain case-exact.'
}

$optInBlocks = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.IfStatementAst] -and
            $node.Extent.Text.Contains('$AllowGameLaunch') -and
            $node.Extent.Text.Contains('manager live game launch requires -AllowGameLaunch')
        }, $true))
Assert-Equal $optInBlocks.Count 1 'Manager live runner must have one fail-closed opt-in block'
$callOperators = @($runner.tokens | Where-Object { $_.Kind -eq 'Ampersand' })
if ($callOperators.Count -eq 0) {
    throw 'Manager live runner must retain bounded child command invocations.'
}
$environmentCalls = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.InvokeMemberExpressionAst] -and
            $node.Member.Extent.Text -ceq 'SetEnvironmentVariable'
        }, $true))
Assert-Equal $environmentCalls.Count 2 `
    'Manager live runner must assign exactly two process-scoped live variables'
$firstCallOffset = ($callOperators | Measure-Object -Property {
        $_.Extent.StartOffset
    } -Minimum).Minimum
$firstEnvironmentOffset = ($environmentCalls | Measure-Object -Property {
        $_.Extent.StartOffset
    } -Minimum).Minimum
if ($optInBlocks[0].Extent.EndOffset -ge $firstCallOffset -or
    $optInBlocks[0].Extent.EndOffset -ge $firstEnvironmentOffset) {
    throw 'Manager live opt-in rejection must precede every call operator and environment mutation.'
}

$testAssignments = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst] -and
            $node.Left.Extent.Text -ceq '$liveTest'
        }, $true))
Assert-Equal $testAssignments.Count 1 'Manager live runner must assign one exact test'
$expectedTest =
    'manager::windows_live_runtime_tests::exact_runtime_runs_through_public_manager_control'
$testValue = $testAssignments[0].Right
if ($testValue -isnot [Management.Automation.Language.CommandExpressionAst] -or
    $testValue.Expression -isnot [Management.Automation.Language.StringConstantExpressionAst] -or
    $testValue.Expression.StringConstantType -ne
        [Management.Automation.Language.StringConstantType]::SingleQuoted -or
    $testValue.Expression.Value -cne $expectedTest) {
    throw 'Manager live runner must select one exact literal ignored test.'
}

$previousStateAssignments = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst] -and
            $node.Left.Extent.Text -ceq '$previousLiveEnvironment'
        }, $true))
Assert-Equal $previousStateAssignments.Count 1 `
    'Manager live runner must capture caller environment exactly once'
if ($previousStateAssignments[0].Extent.StartOffset -ge $firstEnvironmentOffset) {
    throw 'Manager live runner must capture caller environment before mutation.'
}
$guardedTry = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.TryStatementAst] -and
            $null -ne $node.Finally -and
            $node.Finally.Extent.Text.Contains(
                'Restore-ZeusLiveEnvironmentState -State $previousLiveEnvironment'
            )
        }, $true))
Assert-Equal $guardedTry.Count 1 'Manager live runner must restore state from one finally block'
foreach ($call in $environmentCalls) {
    if ($call.Extent.StartOffset -le $guardedTry[0].Body.Extent.StartOffset -or
        $call.Extent.EndOffset -ge $guardedTry[0].Body.Extent.EndOffset -or
        -not $call.Extent.Text.Contains('[EnvironmentVariableTarget]::Process')) {
        throw 'Every manager live environment mutation must be process-scoped inside guarded try.'
    }
}

try {
    Assert-ThrowsLike -Pattern '*manager live game launch requires -AllowGameLaunch*' `
        -Because 'Missing admission must fail before inspecting runtime or invoking Cargo' -Action {
        & $runnerPath -RuntimeRoot 'Z:\definitely-missing-manager-live-runtime'
    }

    $notDirectory = Join-Path ([IO.Path]::GetTempPath()) (
        'zeus-manager-live-contract-' + [Guid]::NewGuid().ToString('D') + '.tmp'
    )
    try {
        New-Item -ItemType File -Path $notDirectory -ErrorAction Stop | Out-Null
        Assert-ThrowsLike -Pattern '*requires a runtime directory*' `
            -Because 'A non-directory runtime must fail before provisioner, Cargo, or Java' -Action {
            & $runnerPath -RuntimeRoot $notDirectory -AllowGameLaunch
        }
    }
    finally {
        if (Test-Path -LiteralPath $notDirectory -PathType Leaf) {
            Remove-Item -LiteralPath $notDirectory -Force
        }
    }
}
finally {
    Restore-ZeusLiveEnvironmentState -State $originalEnvironment
}

$restoredEnvironment = Get-ZeusLiveEnvironmentState
foreach ($prefix in @('runtime_root', 'live_approval')) {
    $present = "${prefix}_present"
    $value = "${prefix}_value"
    Assert-Equal $restoredEnvironment.$present $originalEnvironment.$present `
        "Manager live contract must restore $present"
    if ($originalEnvironment.$present) {
        if ($restoredEnvironment.$value -cne $originalEnvironment.$value) {
            throw "Manager live contract must restore $value case-exactly."
        }
    }
    elseif ($null -ne $restoredEnvironment.$value) {
        throw "Manager live contract must leave $value absent."
    }
}

Write-Output 'PASS: Windows manager control live fail-closed contracts'
