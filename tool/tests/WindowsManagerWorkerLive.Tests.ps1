$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$runnerPath = Join-Path $PSScriptRoot '..\scripts\Invoke-WindowsManagerWorkerLiveTest.ps1'
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

function Get-TokenSignature {
    param([Parameter(Mandatory)] [string] $Text)

    $tokens = $null
    $errors = $null
    [Management.Automation.Language.Parser]::ParseInput(
        $Text,
        [ref] $tokens,
        [ref] $errors
    ) | Out-Null
    if (@($errors).Count -ne 0) {
        throw 'Expected contract expression has PowerShell parse errors.'
    }
    @($tokens | Where-Object {
            $_.Kind -notin @('EndOfInput', 'NewLine', 'LineContinuation', 'Comment')
        } | ForEach-Object {
            $_.Kind.ToString() + ':' + $_.Text
        }) -join '|'
}

function Assert-ExactIfCondition {
    param(
        [Parameter(Mandatory)] $ScriptContract,
        [Parameter(Mandatory)] [string] $BodyLiteral,
        [Parameter(Mandatory)] [string] $ExpectedCondition,
        [Parameter(Mandatory)] [string] $Because,
        [switch] $PassThru
    )

    $expectedSignature = Get-TokenSignature -Text $ExpectedCondition
    $expectedThrowSignature = Get-TokenSignature -Text ("throw '" + $BodyLiteral + "'")
    $candidates = @($ScriptContract.ast.FindAll({
                param($node)
                $node -is [Management.Automation.Language.IfStatementAst] -and
                $node.Extent.Text.Contains($BodyLiteral)
            }, $true))
    $matches = @($candidates | Where-Object {
            if ($_.Clauses.Count -ne 1 -or $null -ne $_.ElseClause -or
                $_.Clauses[0].Item1 -isnot [Management.Automation.Language.PipelineAst] -or
                (Get-TokenSignature -Text $_.Clauses[0].Item1.Extent.Text) -cne
                    $expectedSignature) {
                return $false
            }
            $body = $_.Clauses[0].Item2
            if ($null -ne $body.Traps -or @($body.Statements).Count -ne 1 -or
                $body.Statements[0] -isnot
                    [Management.Automation.Language.ThrowStatementAst]) {
                return $false
            }
            $throw = $body.Statements[0]
            if ($throw.IsRethrow -or
                (Get-TokenSignature -Text $throw.Extent.Text) -cne $expectedThrowSignature) {
                return $false
            }
            $throwLiterals = @($throw.FindAll({
                        param($node)
                        $node -is [Management.Automation.Language.StringConstantExpressionAst]
                    }, $true))
            $throwLiterals.Count -eq 1 -and
            $throwLiterals[0].StringConstantType -eq
                [Management.Automation.Language.StringConstantType]::SingleQuoted -and
            $throwLiterals[0].Value -ceq $BodyLiteral
        })
    Assert-Equal $matches.Count 1 $Because
    if ($PassThru) {
        return $matches[0]
    }
}

function Assert-ExactAssignmentExpression {
    param(
        [Parameter(Mandatory)] $ScriptContract,
        [Parameter(Mandatory)] [string] $Variable,
        [Parameter(Mandatory)] [string] $ExpectedExpression,
        [Parameter(Mandatory)] [string] $Because
    )

    $assignments = @($ScriptContract.ast.FindAll({
                param($node)
                $node -is [Management.Automation.Language.AssignmentStatementAst] -and
                $node.Left.Extent.Text -ceq $Variable
            }, $true))
    Assert-Equal $assignments.Count 1 $Because
    $actualSignature = Get-TokenSignature -Text $assignments[0].Right.Extent.Text
    $expectedSignature = Get-TokenSignature -Text $ExpectedExpression
    if ($actualSignature -cne $expectedSignature) {
        throw "$Because. Exact parsed expression changed."
    }
}

function Assert-ExactArrayAssignment {
    param(
        [Parameter(Mandatory)] $ScriptContract,
        [Parameter(Mandatory)] [string] $Variable,
        [Parameter(Mandatory)] [string[]] $ExpectedElements,
        [Parameter(Mandatory)] [string] $Because
    )

    $assignments = @($ScriptContract.ast.FindAll({
                param($node)
                $node -is [Management.Automation.Language.AssignmentStatementAst] -and
                $node.Left.Extent.Text -ceq $Variable
            }, $true))
    Assert-Equal $assignments.Count 1 $Because
    $right = $assignments[0].Right
    if ($right -isnot [Management.Automation.Language.CommandExpressionAst] -or
        $right.Expression -isnot [Management.Automation.Language.ArrayExpressionAst]) {
        throw "$Because. Expected one literal array expression."
    }
    $statementBlock = $right.Expression.SubExpression
    if ($null -ne $statementBlock.Traps -or
        @($statementBlock.Statements).Count -ne 1 -or
        $statementBlock.Statements[0] -isnot [Management.Automation.Language.PipelineAst]) {
        throw "$Because. Array must contain one expression pipeline."
    }
    $pipeline = $statementBlock.Statements[0]
    if ($pipeline.Background -or $pipeline.PipelineElements.Count -ne 1 -or
        $pipeline.PipelineElements[0] -isnot
            [Management.Automation.Language.CommandExpressionAst] -or
        $pipeline.PipelineElements[0].Redirections.Count -ne 0 -or
        $pipeline.PipelineElements[0].Expression -isnot
            [Management.Automation.Language.ArrayLiteralAst]) {
        throw "$Because. Array pipeline shape changed."
    }
    $elements = @($pipeline.PipelineElements[0].Expression.Elements)
    $actualElements = @($elements | ForEach-Object {
            if ($_ -is [Management.Automation.Language.StringConstantExpressionAst]) {
                if ($_.StringConstantType -ne
                    [Management.Automation.Language.StringConstantType]::SingleQuoted) {
                    throw "$Because. Every literal argument must remain single-quoted."
                }
                'literal:' + $_.Value
            }
            elseif ($_ -is [Management.Automation.Language.VariableExpressionAst]) {
                'variable:' + $_.VariablePath.UserPath
            }
            else {
                throw "$Because. Unrecognized array element type $($_.GetType().Name)."
            }
        })
    Assert-Equal $actualElements.Count $ExpectedElements.Count $Because
    for ($index = 0; $index -lt $ExpectedElements.Count; $index++) {
        if ($actualElements[$index] -cne $ExpectedElements[$index]) {
            throw "$Because. Parsed argument $index changed."
        }
    }
}

if (-not (Test-Path -LiteralPath $runnerPath -PathType Leaf)) {
    throw 'RED: Windows manager worker live runner is missing.'
}

Import-Module $modulePath -Force
$originalEnvironment = Get-ZeusLiveEnvironmentState
$sourceTier = Get-ScriptContract -Path $sourceTierPath -Label 'source tier'
$runner = Get-ScriptContract -Path $runnerPath -Label 'manager worker live runner'

Assert-Equal ([regex]::Matches(
        $sourceTier.text,
        [regex]::Escape('WindowsManagerWorkerLive.Tests.ps1'),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count) 1 'Source tier must invoke the manager worker live contract exactly once'
foreach ($forbidden in @(
        'Invoke-WindowsManagerWorkerLiveTest.ps1',
        'AllowGameLaunch',
        'windows-live-runtime-bridge-v1-approved',
        'temurin-11.0.32+9_microemu-2.0.4_ko402'
    )) {
    Assert-Equal $sourceTier.text.Contains($forbidden) $false `
        'Source tier must not contain manager worker live admission material'
}

$parameterNames = @($runner.ast.ParamBlock.Parameters | ForEach-Object {
        $_.Name.VariablePath.UserPath
    })
Assert-Equal ($parameterNames -join ',') 'RuntimeRoot,AllowGameLaunch' `
    'Manager worker live runner must expose only the runtime root and admission switch'
$switchParameters = @($runner.ast.ParamBlock.Parameters | Where-Object {
        $_.StaticType.FullName -eq 'System.Management.Automation.SwitchParameter'
    })
Assert-Equal $switchParameters.Count 1 'Manager worker live runner must expose one switch'
if ($switchParameters[0].Name.VariablePath.UserPath -cne 'AllowGameLaunch') {
    throw 'Manager worker live admission switch must remain case-exact.'
}

$optInBlock = Assert-ExactIfCondition -ScriptContract $runner `
    -BodyLiteral 'worker live game launch requires -AllowGameLaunch' `
    -ExpectedCondition '-not $AllowGameLaunch' `
    -Because 'Manager worker live runner must have one exact fail-closed opt-in rejection' `
    -PassThru
if (-not [object]::ReferenceEquals($optInBlock.Parent, $runner.ast.EndBlock)) {
    throw 'Manager worker live opt-in rejection must be a top-level runner statement, not nested.'
}

Assert-ExactIfCondition -ScriptContract $runner `
    -BodyLiteral 'Windows manager worker live test requires Windows x64.' `
    -ExpectedCondition '-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne [Runtime.InteropServices.Architecture]::X64' `
    -Because 'Manager worker live runner must retain one exact Windows x64 rejection'

$callOperators = @($runner.tokens | Where-Object { $_.Kind -eq 'Ampersand' })
Assert-Equal $callOperators.Count 3 `
    'Manager worker live runner must retain exactly one preflight and two child invocations'
$environmentCalls = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.InvokeMemberExpressionAst] -and
            $node.Member.Extent.Text -ceq 'SetEnvironmentVariable'
        }, $true))
Assert-Equal $environmentCalls.Count 2 `
    'Manager worker live runner must assign exactly two process-scoped live variables'
$firstCallOffset = ($callOperators | Measure-Object -Property {
        $_.Extent.StartOffset
    } -Minimum).Minimum
$firstEnvironmentOffset = ($environmentCalls | Measure-Object -Property {
        $_.Extent.StartOffset
    } -Minimum).Minimum
if ($optInBlock.Extent.EndOffset -ge $firstCallOffset -or
    $optInBlock.Extent.EndOffset -ge $firstEnvironmentOffset) {
    throw 'Manager worker live opt-in rejection must precede every call operator and environment mutation.'
}

$runtimeItemAssignments = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst] -and
            $node.Left.Extent.Text -ceq '$runtimeItem' -and
            $node.Right.Extent.Text.Contains(
                'Get-Item -LiteralPath $RuntimeRoot -Force -ErrorAction Stop'
            )
        }, $true))
Assert-Equal $runtimeItemAssignments.Count 1 `
    'Manager worker live runner must resolve the caller RuntimeRoot literally'
$runtimeFullAssignments = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst] -and
            $node.Left.Extent.Text -ceq '$runtimeFull' -and
            $node.Right.Extent.Text.Contains('$runtimeItem.FullName') -and
            $node.Right.Extent.Text.Contains('Resolve-Path -LiteralPath')
        }, $true))
Assert-Equal $runtimeFullAssignments.Count 1 `
    'Manager worker live runner must canonicalize the caller runtime directory'

Assert-ExactIfCondition -ScriptContract $runner `
    -BodyLiteral 'Windows manager worker live test requires a fixed local runtime directory.' `
    -ExpectedCondition '[string]::IsNullOrWhiteSpace($runtimeDriveRoot)' `
    -Because 'Manager worker live runner must reject a runtime without a drive root'
Assert-ExactIfCondition -ScriptContract $runner `
    -BodyLiteral 'Windows manager worker live test requires a fixed local runtime directory.' `
    -ExpectedCondition '$runtimeDrive.DriveType -ne [IO.DriveType]::Fixed' `
    -Because 'Manager worker live runner must reject a runtime outside a fixed local drive'
Assert-Equal ([regex]::Matches(
        $runner.text,
        [regex]::Escape('$runtimeDriveRoot = [IO.Path]::GetPathRoot($runtimeFull)'),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count) 1 'Manager worker live runner must derive the drive from canonical RuntimeRoot'
Assert-Equal ([regex]::Matches(
        $runner.text,
        [regex]::Escape('$runtimeDrive = [IO.DriveInfo]::new($runtimeDriveRoot)'),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count) 1 'Manager worker live runner must inspect the canonical runtime drive'

Assert-ExactAssignmentExpression -ScriptContract $runner -Variable '$preflight' `
    -ExpectedExpression '(& $provisioner -RuntimeRoot $runtimeFull -VerifyOnly) | ConvertFrom-Json' `
    -Because 'Manager worker live runner must verify the canonical caller runtime without provisioning'
Assert-ExactIfCondition -ScriptContract $runner `
    -BodyLiteral 'Windows manager worker live exact-runtime preflight did not match the pinned tuple.' `
    -ExpectedCondition "`$preflight.status -ne 'verified' -or `$preflight.runtime_id -cne 'windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402' -or `$preflight.jre_file_count -ne 337" `
    -Because 'Manager worker live runner must require verified pinned ID and 337-file preflight'

$testAssignments = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst] -and
            $node.Left.Extent.Text -ceq '$liveTest'
        }, $true))
Assert-Equal $testAssignments.Count 1 'Manager worker live runner must assign one exact test'
$expectedTest =
    'manager::worker::windows_live_runtime_tests::exact_runtime_runs_through_public_manager_worker'
$testValue = $testAssignments[0].Right
if ($testValue -isnot [Management.Automation.Language.CommandExpressionAst] -or
    $testValue.Expression -isnot [Management.Automation.Language.StringConstantExpressionAst] -or
    $testValue.Expression.StringConstantType -ne
        [Management.Automation.Language.StringConstantType]::SingleQuoted -or
    $testValue.Expression.Value -cne $expectedTest) {
    throw 'Manager worker live runner must select one exact literal ignored test.'
}

Assert-ExactArrayAssignment -ScriptContract $runner -Variable '$listArguments' `
    -ExpectedElements @(
        'literal:test', 'literal:--locked', 'literal:--manifest-path',
        'variable:manifestPath', 'literal:-p', 'literal:zeus-core', 'literal:--lib',
        'variable:liveTest', 'literal:--', 'literal:--ignored', 'literal:--list'
    ) `
    -Because 'Manager worker live listing must use one exact ignored-test argument array'
Assert-ExactArrayAssignment -ScriptContract $runner -Variable '$runArguments' `
    -ExpectedElements @(
        'literal:test', 'literal:--locked', 'literal:--manifest-path',
        'variable:manifestPath', 'literal:-p', 'literal:zeus-core', 'literal:--lib',
        'variable:liveTest', 'literal:--', 'literal:--ignored', 'literal:--exact',
        'literal:--test-threads=1', 'literal:--nocapture'
    ) `
    -Because 'Manager worker live execution must use one exact serialized argument array'
Assert-Equal ([regex]::Matches(
        $runner.text,
        [regex]::Escape('& $powerShell -NoProfile -File $cargoWrapper'),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count) 2 'Manager worker live runner must invoke Cargo through exactly two child processes'

Assert-ExactAssignmentExpression -ScriptContract $runner -Variable '$listOutput' `
    -ExpectedExpression '@(& $powerShell -NoProfile -File $cargoWrapper @listArguments 2>&1)' `
    -Because 'Manager worker live runner must invoke the exact listing array once'
Assert-ExactAssignmentExpression -ScriptContract $runner -Variable '$runOutput' `
    -ExpectedExpression '@(& $powerShell -NoProfile -File $cargoWrapper @runArguments 2>&1)' `
    -Because 'Manager worker live runner must invoke the exact run array once'
Assert-Equal ([regex]::Matches(
        $runner.text,
        [regex]::Escape(
            'Assert-ZeusExactTestListing -Output $listOutput -ExpectedTestName $liveTest'
        ),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count) 1 'Manager worker live runner must assert one exact listed test'
Assert-Equal ([regex]::Matches(
        $runner.text,
        [regex]::Escape(
            'Assert-ZeusSingleExactTestResult -Output $runOutput -ExpectedTestName $liveTest'
        ),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count) 1 'Manager worker live runner must assert one exact passing test result'

$runtimeEnvironmentAssignments = @($environmentCalls | Where-Object {
        $_.Extent.Text.Contains("'ZEUS_EXACT_RUNTIME_ROOT'") -and
        $_.Extent.Text.Contains('$runtimeFull')
    })
Assert-Equal $runtimeEnvironmentAssignments.Count 1 `
    'Manager worker live runner must pass canonical caller RuntimeRoot to the exact test'

$previousStateAssignments = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst] -and
            $node.Left.Extent.Text -ceq '$previousLiveEnvironment'
        }, $true))
Assert-Equal $previousStateAssignments.Count 1 `
    'Manager worker live runner must capture caller environment exactly once'
Assert-ExactAssignmentExpression -ScriptContract $runner `
    -Variable '$previousLiveEnvironment' `
    -ExpectedExpression 'Get-ZeusLiveEnvironmentState' `
    -Because 'Manager worker live runner must capture caller environment with one zero-argument command'
if (-not [object]::ReferenceEquals(
        $previousStateAssignments[0].Parent,
        $runner.ast.EndBlock
    )) {
    throw 'Manager worker live environment capture must be unconditional at script scope.'
}
if ($previousStateAssignments[0].Extent.StartOffset -ge $firstEnvironmentOffset) {
    throw 'Manager worker live runner must capture caller environment before mutation.'
}
$guardedTry = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.TryStatementAst]
        }, $true))
Assert-Equal $guardedTry.Count 1 'Manager worker live runner must contain one guarded execution block'
if ($null -eq $guardedTry[0].Finally -or $null -ne $guardedTry[0].Finally.Traps -or
    @($guardedTry[0].Finally.Statements).Count -ne 1 -or
    $guardedTry[0].Finally.Statements[0] -isnot
        [Management.Automation.Language.PipelineAst] -or
    (Get-TokenSignature -Text $guardedTry[0].Finally.Statements[0].Extent.Text) -cne
        (Get-TokenSignature -Text 'Restore-ZeusLiveEnvironmentState -State $previousLiveEnvironment')) {
    throw 'Manager worker live runner must unconditionally restore state once from finally.'
}
foreach ($call in $environmentCalls) {
    if ($call.Extent.StartOffset -le $guardedTry[0].Body.Extent.StartOffset -or
        $call.Extent.EndOffset -ge $guardedTry[0].Body.Extent.EndOffset -or
        -not $call.Extent.Text.Contains('[EnvironmentVariableTarget]::Process')) {
        throw 'Every manager worker live environment mutation must be process-scoped inside guarded try.'
    }
}

try {
    Assert-ThrowsLike -Pattern '*worker live game launch requires -AllowGameLaunch*' `
        -Because 'Missing admission must fail before inspecting runtime or invoking Cargo' -Action {
        & $runnerPath -RuntimeRoot 'Z:\definitely-missing-manager-worker-live-runtime'
    }

    $notDirectory = Join-Path ([IO.Path]::GetTempPath()) (
        'zeus-manager-worker-live-contract-' + [Guid]::NewGuid().ToString('D') + '.tmp'
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
        "Manager worker live contract must restore $present"
    if ($originalEnvironment.$present) {
        if ($restoredEnvironment.$value -cne $originalEnvironment.$value) {
            throw "Manager worker live contract must restore $value case-exactly."
        }
    }
    elseif ($null -ne $restoredEnvironment.$value) {
        throw "Manager worker live contract must leave $value absent."
    }
}

Write-Output 'PASS: Windows manager worker live fail-closed contracts'
