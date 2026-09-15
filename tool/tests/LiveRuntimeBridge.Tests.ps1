$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$runnerPath = Join-Path $PSScriptRoot '..\scripts\Invoke-LiveRuntimeBridgeTests.ps1'
$modulePath = Join-Path $PSScriptRoot '..\scripts\Zeus.TestTiers.psm1'
$sourceTierPath = Join-Path $PSScriptRoot '..\scripts\Invoke-SourceTests.ps1'
$exactTierPath = Join-Path $PSScriptRoot '..\scripts\Invoke-ExactRuntimeTests.ps1'

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

function Assert-LiveEnvironmentStateEqual {
    param(
        [Parameter(Mandatory)] [pscustomobject] $Expected,
        [Parameter(Mandatory)] [string] $Because
    )

    $actual = Get-ZeusLiveEnvironmentState
    foreach ($prefix in @('runtime_root', 'live_approval')) {
        $presentProperty = "${prefix}_present"
        $valueProperty = "${prefix}_value"
        Assert-Equal $actual.$presentProperty $Expected.$presentProperty `
            "${Because}: $presentProperty"
        if ($Expected.$presentProperty) {
            if ($actual.$valueProperty -cne $Expected.$valueProperty) {
                throw "${Because}: $valueProperty must be case-exact."
            }
        }
        elseif ($null -ne $actual.$valueProperty) {
            throw "${Because}: $valueProperty must remain absent."
        }
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

function New-ValidLivePerformanceRecord {
    [pscustomobject] [ordered] @{
        schema_version = 1
        runtime_id = 'windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402'
        concurrent_sessions = 4
        stabilization_seconds = 15
        observation_seconds = 60
        sample_interval_seconds = 5
        samples_per_session = 13
        representative_of_1gib_target = $false
        capacity_rejection_confirmed = $true
        all_windows_responsive = $true
        cleanup_confirmed = $true
        start_to_window_ms = @(11, 22, 33, 44)
        per_session = @(
            [pscustomobject] [ordered] @{
                index = 1
                max_working_set_bytes = 100
                final_working_set_bytes = 90
                max_private_bytes = 200
                final_private_bytes = 180
                max_handle_count = 10
                cpu_percent_one_core_x100 = 101
            },
            [pscustomobject] [ordered] @{
                index = 2
                max_working_set_bytes = 100
                final_working_set_bytes = 90
                max_private_bytes = 200
                final_private_bytes = 180
                max_handle_count = 10
                cpu_percent_one_core_x100 = 102
            },
            [pscustomobject] [ordered] @{
                index = 3
                max_working_set_bytes = 100
                final_working_set_bytes = 90
                max_private_bytes = 200
                final_private_bytes = 180
                max_handle_count = 10
                cpu_percent_one_core_x100 = 103
            },
            [pscustomobject] [ordered] @{
                index = 4
                max_working_set_bytes = 100
                final_working_set_bytes = 90
                max_private_bytes = 200
                final_private_bytes = 180
                max_handle_count = 10
                cpu_percent_one_core_x100 = 104
            }
        )
        aggregate = [pscustomobject] [ordered] @{
            first_working_set_bytes = 300
            max_working_set_bytes = 400
            final_working_set_bytes = 350
            working_set_growth_bytes = 50
            first_private_bytes = 600
            max_private_bytes = 800
            final_private_bytes = 700
            private_growth_bytes = 100
            max_handle_count = 40
            cpu_percent_one_core_x100 = 410
        }
    }
}

function Copy-LivePerformanceRecord {
    param([Parameter(Mandatory)] [object] $Record)

    $Record | ConvertTo-Json -Depth 8 -Compress | ConvertFrom-Json
}

function ConvertTo-LivePerformanceLine {
    param([Parameter(Mandatory)] [object] $Record)

    'ZEUS_LIVE_PERF_V1=' + ($Record | ConvertTo-Json -Depth 8 -Compress)
}

function Assert-PerformanceRecordRejected {
    param(
        [Parameter(Mandatory)] [object] $Record,
        [Parameter(Mandatory)] [string] $Because
    )

    Assert-ThrowsLike -Pattern '*live performance record*' -Because $Because -Action {
        Read-ZeusLivePerformanceRecord -Output @(ConvertTo-LivePerformanceLine -Record $Record)
    }
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

function Get-NormalizedPrivacyLabel {
    param([Parameter(Mandatory)] [string] $Label)

    $Label.ToLowerInvariant() -replace '[^a-z0-9]', ''
}

$missingContracts = @()
if (-not (Test-Path -LiteralPath $runnerPath -PathType Leaf)) {
    $missingContracts += 'runner'
}
Import-Module $modulePath -Force
if (-not (Get-Command Read-ZeusLivePerformanceRecord -ErrorAction SilentlyContinue)) {
    $missingContracts += 'parser'
}
if (-not (Get-Command Get-ZeusLiveEnvironmentState -ErrorAction SilentlyContinue) -or
    -not (Get-Command Restore-ZeusLiveEnvironmentState -ErrorAction SilentlyContinue)) {
    $missingContracts += 'environment restoration'
}
if ($missingContracts.Count -ne 0) {
    throw "RED: live runtime bridge contract is missing: $($missingContracts -join ', ')"
}

$sourceTier = Get-ScriptContract -Path $sourceTierPath -Label 'source tier'
$exactTier = Get-ScriptContract -Path $exactTierPath -Label 'exact tier'
$runner = Get-ScriptContract -Path $runnerPath -Label 'live runner'

$wrongCaseRuntimeId = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$wrongCaseRuntimeId.runtime_id = 'Windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402'
Assert-PerformanceRecordRejected $wrongCaseRuntimeId `
    'Wrong-case fixed runtime ID must fail closed'

$sourceContractReference = 'LiveRuntimeBridge.Tests.ps1'
Assert-Equal ([regex]::Matches(
        $sourceTier.text,
        [regex]::Escape($sourceContractReference),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count) 1 'Source tier must invoke the live contract exactly once'
foreach ($forbidden in @(
        'Invoke-LiveRuntimeBridgeTests.ps1',
        'AllowGameLaunch',
        'windows-live-runtime-bridge-v1-approved',
        'temurin-11.0.32+9_microemu-2.0.4_ko402'
    )) {
    Assert-Equal $sourceTier.text.Contains($forbidden) $false `
        'Source tier must not reference live admission or exact runtime material'
}
foreach ($forbidden in @(
        'Invoke-LiveRuntimeBridgeTests.ps1',
        'AllowGameLaunch',
        'ZEUS_LIVE_RUNTIME_BRIDGE',
        'windows-live-runtime-bridge-v1-approved'
    )) {
    Assert-Equal $exactTier.text.Contains($forbidden) $false `
        'Exact-static tier must not reference the live runner or admission token'
}

$switchParameters = @($runner.ast.ParamBlock.Parameters | Where-Object {
        $_.StaticType.FullName -eq 'System.Management.Automation.SwitchParameter'
    })
$runnerParameterNames = @($runner.ast.ParamBlock.Parameters | ForEach-Object {
        $_.Name.VariablePath.UserPath
    })
Assert-Equal ($runnerParameterNames -join ',') 'RuntimeRoot,AllowGameLaunch' `
    'Live runner parameters must remain the runtime root plus sole admission switch'
Assert-Equal $switchParameters.Count 1 'Live runner must expose one admission switch'
if ($switchParameters[0].Name.VariablePath.UserPath -cne 'AllowGameLaunch') {
    throw 'Live runner admission switch must remain case-exact.'
}
$runtimeIdComparisons = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.BinaryExpressionAst] -and
            $node.Left.Extent.Text -ceq '$preflight.runtime_id' -and
            $node.Right -is [Management.Automation.Language.StringConstantExpressionAst] -and
            $node.Right.Value -ceq
                'windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402'
        }, $true))
Assert-Equal $runtimeIdComparisons.Count 1 `
    'Live runner must perform one fixed preflight runtime ID comparison'
if ($runtimeIdComparisons[0].Operator -ne
        [Management.Automation.Language.TokenKind]::Cne) {
    throw 'Live runner preflight runtime ID comparison must be case-sensitive.'
}
$optInBlocks = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.IfStatementAst] -and
            $node.Extent.Text.Contains('$AllowGameLaunch') -and
            $node.Extent.Text.Contains('live game launch requires -AllowGameLaunch')
        }, $true))
Assert-Equal $optInBlocks.Count 1 'Live runner must have one fail-closed opt-in block'
$callOperators = @($runner.tokens | Where-Object { $_.Kind -eq 'Ampersand' })
if ($callOperators.Count -eq 0) {
    throw 'Live runner must retain bounded child command invocations.'
}
$environmentCalls = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.InvokeMemberExpressionAst] -and
            $node.Member.Extent.Text -ceq 'SetEnvironmentVariable'
        }, $true))
Assert-Equal $environmentCalls.Count 2 `
    'Live runner must assign exactly the two process-scoped live variables'
$firstCallOffset = ($callOperators | Measure-Object -Property {
        $_.Extent.StartOffset
    } -Minimum).Minimum
$firstEnvironmentOffset = ($environmentCalls | Measure-Object -Property {
        $_.Extent.StartOffset
    } -Minimum).Minimum
if ($optInBlocks[0].Extent.EndOffset -ge $firstCallOffset -or
    $optInBlocks[0].Extent.EndOffset -ge $firstEnvironmentOffset) {
    throw 'Live opt-in rejection must precede every call operator and environment mutation.'
}

$liveAssignments = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst] -and
            $node.Left.Extent.Text -ceq '$liveTests'
        }, $true))
Assert-Equal $liveAssignments.Count 1 'Live runner must assign one fixed liveTests table'
$right = $liveAssignments[0].Right
if ($right -isnot [Management.Automation.Language.CommandExpressionAst] -or
    $right.Expression -isnot [Management.Automation.Language.ArrayExpressionAst]) {
    throw 'Live runner liveTests table must be one literal array.'
}
$tableStatements = @($right.Expression.SubExpression.Statements)
if ($tableStatements.Count -ne 1 -or
    @($tableStatements[0].PipelineElements).Count -ne 1 -or
    $tableStatements[0].PipelineElements[0] -isnot
        [Management.Automation.Language.CommandExpressionAst] -or
    $tableStatements[0].PipelineElements[0].Expression -isnot
        [Management.Automation.Language.ArrayLiteralAst]) {
    throw 'Live runner liveTests table must contain only literal entries.'
}
$tableElements = @($tableStatements[0].PipelineElements[0].Expression.Elements)
$expectedLiveTests = @(
    'session_supervisor::windows_live_runtime_tests::exact_runtime_launches_through_production_supervisor_and_hard_stops',
    'session_supervisor::windows_live_runtime_tests::four_exact_profiles_stay_isolated_within_performance_bounds',
    'session_supervisor::windows_live_runtime_tests::real_java_exits_when_supervisor_owner_is_terminated'
)
Assert-Equal $tableElements.Count $expectedLiveTests.Count `
    'Live runner must select exactly three top-level tests'
for ($index = 0; $index -lt $expectedLiveTests.Count; $index++) {
    if ($tableElements[$index] -isnot
            [Management.Automation.Language.StringConstantExpressionAst] -or
        $tableElements[$index].StringConstantType -ne
            [Management.Automation.Language.StringConstantType]::SingleQuoted -or
        $tableElements[$index].Value -cne $expectedLiveTests[$index]) {
        throw 'Live runner top-level test table must remain exact, literal, and ordered.'
    }
}
Assert-Equal $runner.text.Contains('live_runtime_parent_crash_child') $false `
    'Child-only parent-crash test must never be a runner top-level selection'

$previousStateAssignments = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.AssignmentStatementAst] -and
            $node.Left.Extent.Text -ceq '$previousLiveEnvironment'
        }, $true))
Assert-Equal $previousStateAssignments.Count 1 `
    'Live runner must capture caller environment exactly once'
if ($previousStateAssignments[0].Extent.StartOffset -ge $firstEnvironmentOffset) {
    throw 'Live runner must capture caller environment before mutation.'
}
$guardedTry = @($runner.ast.FindAll({
            param($node)
            $node -is [Management.Automation.Language.TryStatementAst] -and
            $null -ne $node.Finally -and
            $node.Finally.Extent.Text.Contains(
                'Restore-ZeusLiveEnvironmentState -State $previousLiveEnvironment'
            )
        }, $true))
Assert-Equal $guardedTry.Count 1 'Live runner must restore caller state from one finally block'
Assert-Equal ([regex]::Matches(
        $guardedTry[0].Finally.Extent.Text,
        [regex]::Escape('Restore-ZeusLiveEnvironmentState -State $previousLiveEnvironment'),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count) 1 'Live runner finally must restore the captured state exactly once'
foreach ($call in $environmentCalls) {
    if ($call.Extent.StartOffset -le $guardedTry[0].Body.Extent.StartOffset -or
        $call.Extent.EndOffset -ge $guardedTry[0].Body.Extent.EndOffset) {
        throw 'Every live environment assignment must remain inside the guarded try.'
    }
    if (-not $call.Extent.Text.Contains('[EnvironmentVariableTarget]::Process')) {
        throw 'Every live environment assignment must remain process-scoped.'
    }
}
foreach ($name in @('ZEUS_EXACT_RUNTIME_ROOT', 'ZEUS_LIVE_RUNTIME_BRIDGE')) {
    $matchingCalls = @($environmentCalls | Where-Object { $_.Extent.Text.Contains("'$name'") })
    Assert-Equal $matchingCalls.Count 1 `
        'Guarded try must assign each approved process-scoped live variable exactly once'
}

$contractOriginalEnvironment = Get-ZeusLiveEnvironmentState
try {
    $rejectedLaunchEnvironment = Get-ZeusLiveEnvironmentState
    try {
        [Environment]::SetEnvironmentVariable(
            'ZEUS_LIVE_RUNTIME_BRIDGE',
            'caller-value-must-survive-rejection',
            [EnvironmentVariableTarget]::Process
        )
        Assert-ThrowsLike -Pattern '*live game launch requires -AllowGameLaunch*' `
            -Because 'The live runner must reject before inspecting a nonexistent runtime root' -Action {
                & $runnerPath -RuntimeRoot 'Z:\definitely-missing-live-runtime'
            }
        Assert-Equal ([Environment]::GetEnvironmentVariable(
                'ZEUS_LIVE_RUNTIME_BRIDGE',
                [EnvironmentVariableTarget]::Process
            )) 'caller-value-must-survive-rejection' `
            'Rejected launch must not replace the caller live-approval value'
    }
    finally {
        Restore-ZeusLiveEnvironmentState -State $rejectedLaunchEnvironment
    }
    Assert-LiveEnvironmentStateEqual -Expected $rejectedLaunchEnvironment `
        -Because 'Rejected-launch coverage must restore its exact incoming environment'

    $environmentNames = @('ZEUS_EXACT_RUNTIME_ROOT', 'ZEUS_LIVE_RUNTIME_BRIDGE')
    $environmentCoverageOriginal = Get-ZeusLiveEnvironmentState
    foreach ($name in $environmentNames) {
        Remove-Item -LiteralPath "Env:$name" -Force -ErrorAction SilentlyContinue
    }
    $absentState = Get-ZeusLiveEnvironmentState
    [Environment]::SetEnvironmentVariable(
        'ZEUS_EXACT_RUNTIME_ROOT',
        'synthetic-runtime-replacement',
        [EnvironmentVariableTarget]::Process
    )
    [Environment]::SetEnvironmentVariable(
        'ZEUS_LIVE_RUNTIME_BRIDGE',
        'synthetic-approval-replacement',
        [EnvironmentVariableTarget]::Process
    )
    Restore-ZeusLiveEnvironmentState -State $absentState
    foreach ($name in $environmentNames) {
        Assert-Equal (Test-Path -LiteralPath "Env:$name") $false `
            "Restoration must keep an originally absent $name absent"
    }

    foreach ($name in $environmentNames) {
        [Environment]::SetEnvironmentVariable(
            $name,
            [string]::Empty,
            [EnvironmentVariableTarget]::Process
        )
    }
    if (@($environmentNames | Where-Object {
                -not (Test-Path -LiteralPath "Env:$_")
            }).Count -eq 0) {
        $emptyState = Get-ZeusLiveEnvironmentState
        foreach ($name in $environmentNames) {
            [Environment]::SetEnvironmentVariable(
                $name,
                'synthetic-nonempty-replacement',
                [EnvironmentVariableTarget]::Process
            )
        }
        Restore-ZeusLiveEnvironmentState -State $emptyState
        Assert-LiveEnvironmentStateEqual -Expected $emptyState `
            -Because 'Restoration must preserve representable present-empty caller state'
    }
    else {
        Restore-ZeusLiveEnvironmentState -State $absentState
    }

    [Environment]::SetEnvironmentVariable(
        'ZEUS_EXACT_RUNTIME_ROOT',
        'caller-runtime-value',
        [EnvironmentVariableTarget]::Process
    )
    [Environment]::SetEnvironmentVariable(
        'ZEUS_LIVE_RUNTIME_BRIDGE',
        'caller-approval-value',
        [EnvironmentVariableTarget]::Process
    )
    $presentState = Get-ZeusLiveEnvironmentState
    try {
        [Environment]::SetEnvironmentVariable(
            'ZEUS_EXACT_RUNTIME_ROOT',
            'synthetic-runtime-replacement',
            [EnvironmentVariableTarget]::Process
        )
        [Environment]::SetEnvironmentVariable(
            'ZEUS_LIVE_RUNTIME_BRIDGE',
            'synthetic-approval-replacement',
            [EnvironmentVariableTarget]::Process
        )
        throw 'synthetic child command failure'
    }
    catch {
        Assert-Equal $_.Exception.Message 'synthetic child command failure' `
            'The synthetic failure must reach the runner-style finally path'
    }
    finally {
        Restore-ZeusLiveEnvironmentState -State $presentState
    }
    Assert-Equal ([Environment]::GetEnvironmentVariable(
            'ZEUS_EXACT_RUNTIME_ROOT',
            [EnvironmentVariableTarget]::Process
        )) 'caller-runtime-value' 'Failure cleanup must restore the prior runtime-root value'
    Assert-Equal ([Environment]::GetEnvironmentVariable(
            'ZEUS_LIVE_RUNTIME_BRIDGE',
            [EnvironmentVariableTarget]::Process
        )) 'caller-approval-value' 'Failure cleanup must restore the prior approval value'
    foreach ($name in $environmentNames) {
        Assert-Equal (Test-Path -LiteralPath "Env:$name") $true `
            "Restoration must keep an originally present $name present"
    }
    Restore-ZeusLiveEnvironmentState -State $environmentCoverageOriginal
    Assert-LiveEnvironmentStateEqual -Expected $environmentCoverageOriginal `
        -Because 'Environment coverage must restore its exact incoming environment'

Assert-ThrowsLike -Pattern '*live performance record*' `
    -Because 'Zero prefixed lines must fail closed' -Action {
        Read-ZeusLivePerformanceRecord -Output @('ordinary output')
    }
$validLine = ConvertTo-LivePerformanceLine -Record (New-ValidLivePerformanceRecord)
Assert-ThrowsLike -Pattern '*live performance record*' `
    -Because 'Two prefixed lines must fail closed' -Action {
        Read-ZeusLivePerformanceRecord -Output @($validLine, $validLine)
    }
Assert-ThrowsLike -Pattern '*live performance record*' `
    -Because 'Invalid JSON must fail closed' -Action {
        Read-ZeusLivePerformanceRecord -Output @('ZEUS_LIVE_PERF_V1={not-json')
    }
Assert-ThrowsLike -Pattern '*live performance record*' `
    -Because 'An empty JSON payload must fail closed' -Action {
        Read-ZeusLivePerformanceRecord -Output @('ZEUS_LIVE_PERF_V1=')
    }
Assert-ThrowsLike -Pattern '*live performance record*' `
    -Because 'An oversized JSON payload must fail closed' -Action {
        Read-ZeusLivePerformanceRecord -Output @(('ZEUS_LIVE_PERF_V1=' + ('x' * 65537)))
    }

$wrongFixedField = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$wrongFixedField.schema_version = 2
Assert-PerformanceRecordRejected $wrongFixedField 'A wrong fixed field must fail closed'

$wrongSessionCount = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$wrongSessionCount.per_session = @($wrongSessionCount.per_session | Select-Object -First 3)
Assert-PerformanceRecordRejected $wrongSessionCount 'A wrong per-session count must fail closed'

$wrongSessionIndex = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$wrongSessionIndex.per_session[1].index = 1
Assert-PerformanceRecordRejected $wrongSessionIndex 'Duplicate or missing session indices must fail closed'

$extraTopLevel = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$extraTopLevel | Add-Member -NotePropertyName unexpected -NotePropertyValue 1
Assert-PerformanceRecordRejected $extraTopLevel 'An extra top-level field must fail closed'

$wrongCaseTopLevel = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$schemaVersion = $wrongCaseTopLevel.schema_version
$wrongCaseTopLevel.PSObject.Properties.Remove('schema_version')
$wrongCaseTopLevel | Add-Member -NotePropertyName Schema_version -NotePropertyValue $schemaVersion
Assert-PerformanceRecordRejected $wrongCaseTopLevel `
    'A wrong-case top-level property must fail the exact property-name contract'

$extraPerSession = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$extraPerSession.per_session[0] | Add-Member -NotePropertyName unexpected -NotePropertyValue 1
Assert-PerformanceRecordRejected $extraPerSession 'An extra per-session field must fail closed'

$extraAggregate = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$extraAggregate.aggregate | Add-Member -NotePropertyName unexpected -NotePropertyValue 1
Assert-PerformanceRecordRejected $extraAggregate 'An extra aggregate field must fail closed'

$ceilingCases = @(
    [pscustomobject] @{
        because = 'Per-session working-set ceiling exceeded by one must fail closed'
        mutate = { param($record) $record.per_session[0].max_working_set_bytes = 268435457 }
    },
    [pscustomobject] @{
        because = 'Per-session private-byte ceiling exceeded by one must fail closed'
        mutate = { param($record) $record.per_session[0].max_private_bytes = 268435457 }
    },
    [pscustomobject] @{
        because = 'Per-session handle ceiling exceeded by one must fail closed'
        mutate = { param($record) $record.per_session[0].max_handle_count = 1201 }
    },
    [pscustomobject] @{
        because = 'Aggregate working-set ceiling exceeded by one must fail closed'
        mutate = {
            param($record)
            foreach ($session in $record.per_session) {
                $session.max_working_set_bytes = 201326593
            }
            $record.aggregate.max_working_set_bytes = 805306369
        }
    },
    [pscustomobject] @{
        because = 'Aggregate private-byte ceiling exceeded by one must fail closed'
        mutate = {
            param($record)
            foreach ($session in $record.per_session) {
                $session.max_private_bytes = 201326593
            }
            $record.aggregate.max_private_bytes = 805306369
        }
    },
    [pscustomobject] @{
        because = 'Aggregate handle ceiling exceeded by one must fail closed'
        mutate = {
            param($record)
            foreach ($session in $record.per_session) {
                $session.max_handle_count = 1200
            }
            $record.aggregate.max_handle_count = 4801
        }
    },
    [pscustomobject] @{
        because = 'Aggregate CPU ceiling exceeded by one must fail closed'
        mutate = {
            param($record)
            foreach ($session in $record.per_session) {
                $session.cpu_percent_one_core_x100 = 2501
            }
            $record.aggregate.cpu_percent_one_core_x100 = 10001
        }
    },
    [pscustomobject] @{
        because = 'Aggregate working-set growth ceiling exceeded by one must fail closed'
        mutate = {
            param($record)
            foreach ($session in $record.per_session) {
                $session.max_working_set_bytes = 33554433
            }
            $record.aggregate.first_working_set_bytes = 0
            $record.aggregate.max_working_set_bytes = 134217729
            $record.aggregate.final_working_set_bytes = 134217729
            $record.aggregate.working_set_growth_bytes = 134217729
        }
    },
    [pscustomobject] @{
        because = 'Aggregate private growth ceiling exceeded by one must fail closed'
        mutate = {
            param($record)
            foreach ($session in $record.per_session) {
                $session.max_private_bytes = 33554433
            }
            $record.aggregate.first_private_bytes = 0
            $record.aggregate.max_private_bytes = 134217729
            $record.aggregate.final_private_bytes = 134217729
            $record.aggregate.private_growth_bytes = 134217729
        }
    }
)
foreach ($case in $ceilingCases) {
    $invalidRecord = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
    $mutate = $case.mutate
    & $mutate $invalidRecord
    Assert-PerformanceRecordRejected $invalidRecord $case.because
}

$negativeMeasurement = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$negativeMeasurement.start_to_window_ms[0] = -1
Assert-PerformanceRecordRejected $negativeMeasurement 'A negative unsigned measurement must fail closed'

$fractionalFixed = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$fractionalFixed.schema_version = 1.5
Assert-PerformanceRecordRejected $fractionalFixed 'A fractional fixed field must fail closed'

$fractionalStart = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$fractionalStart.start_to_window_ms[0] = 11.5
Assert-PerformanceRecordRejected $fractionalStart `
    'A fractional start-to-window measurement must fail closed'

$fractionalSessionMetric = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$fractionalSessionMetric.per_session[0].final_working_set_bytes = 89.5
Assert-PerformanceRecordRejected $fractionalSessionMetric `
    'A fractional per-session metric must fail closed'

$fractionalSessionIndex = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$fractionalSessionIndex.per_session[0].index = 1.5
Assert-PerformanceRecordRejected $fractionalSessionIndex `
    'A fractional per-session index must fail closed'

$fractionalAggregateMetric = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$fractionalAggregateMetric.aggregate.cpu_percent_one_core_x100 = 410.5
Assert-PerformanceRecordRejected $fractionalAggregateMetric `
    'A fractional unsigned aggregate metric must fail closed'

$fractionalSignedGrowth = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$fractionalSignedGrowth.aggregate.final_working_set_bytes = 350.5
$fractionalSignedGrowth.aggregate.working_set_growth_bytes = 50.5
Assert-PerformanceRecordRejected $fractionalSignedGrowth `
    'A fractional signed growth measurement must fail closed even when arithmetic is exact'

$unsignedOutOfRange = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$unsignedOutOfRange.start_to_window_ms[0] = 1E+20
Assert-PerformanceRecordRejected $unsignedOutOfRange `
    'An integral exponent value above the u64 domain must fail closed'

$wrongGrowth = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$wrongGrowth.aggregate.working_set_growth_bytes = 49
Assert-PerformanceRecordRejected $wrongGrowth 'Growth must equal signed final minus first'

$aggregateAboveSessionSum = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$aggregateAboveSessionSum.aggregate.max_working_set_bytes = 401
Assert-PerformanceRecordRejected $aggregateAboveSessionSum `
    'Aggregate maximum above the checked sum of session maxima must fail closed'

$privacyPropertyCases = @(
    [pscustomobject] @{ token = 'pid'; candidate = 'PID' },
    [pscustomobject] @{ token = 'creation'; candidate = 'Creation-Time' },
    [pscustomobject] @{ token = 'path'; candidate = 'runtime.path' },
    [pscustomobject] @{ token = 'argv'; candidate = 'ArgV' },
    [pscustomobject] @{ token = 'environment'; candidate = 'processEnvironment' },
    [pscustomobject] @{ token = 'username'; candidate = 'User_Name' },
    [pscustomobject] @{ token = 'window_title'; candidate = 'WindowTitle' },
    [pscustomobject] @{ token = 'hostname'; candidate = 'HOST-NAME' },
    [pscustomobject] @{ token = 'machine_id'; candidate = 'machine.id' }
)
$schemaKinds = @('top-level', 'per-session', 'aggregate')
for ($caseIndex = 0; $caseIndex -lt $privacyPropertyCases.Count; $caseIndex++) {
    $case = $privacyPropertyCases[$caseIndex]
    $normalizedToken = Get-NormalizedPrivacyLabel $case.token
    $normalizedCandidate = Get-NormalizedPrivacyLabel $case.candidate
    if (-not $normalizedCandidate.Contains($normalizedToken)) {
        throw "Synthetic privacy label $caseIndex does not cover its normalized token."
    }
    for ($kindIndex = 0; $kindIndex -lt $schemaKinds.Count; $kindIndex++) {
        $invalidRecord = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
        $target = switch ($schemaKinds[$kindIndex]) {
            'top-level' { $invalidRecord }
            'per-session' { $invalidRecord.per_session[0] }
            'aggregate' { $invalidRecord.aggregate }
        }
        $target | Add-Member -NotePropertyName $case.candidate -NotePropertyValue 1
        Assert-PerformanceRecordRejected $invalidRecord `
            "Privacy property case $caseIndex at schema kind $kindIndex must fail closed"
    }
}

$wrongRuntimeId = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$wrongRuntimeId.runtime_id = 'unexpected-runtime-id'
Assert-PerformanceRecordRejected $wrongRuntimeId 'Unexpected runtime ID string must fail closed'

$stringInTopLevelNumeric = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$stringInTopLevelNumeric.schema_version = 'not-a-number'
Assert-PerformanceRecordRejected $stringInTopLevelNumeric `
    'A string in a top-level numeric field must fail closed'

$stringInTopLevelBoolean = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$stringInTopLevelBoolean.cleanup_confirmed = 'not-a-boolean'
Assert-PerformanceRecordRejected $stringInTopLevelBoolean `
    'A string in a top-level boolean field must fail closed'

$stringInSessionNumeric = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$stringInSessionNumeric.per_session[0].max_handle_count = 'not-a-number'
Assert-PerformanceRecordRejected $stringInSessionNumeric `
    'A string in a per-session numeric field must fail closed'

$stringInAggregateNumeric = Copy-LivePerformanceRecord (New-ValidLivePerformanceRecord)
$stringInAggregateNumeric.aggregate.working_set_growth_bytes = 'not-a-number'
Assert-PerformanceRecordRejected $stringInAggregateNumeric `
    'A string in an aggregate signed field must fail closed'

$valid = Read-ZeusLivePerformanceRecord -Output @(
    'cargo diagnostic',
    (ConvertTo-LivePerformanceLine -Record (New-ValidLivePerformanceRecord)),
    'test result: ok'
)
Assert-Equal $valid.schema_version 1 'Valid record must retain schema version'
Assert-Equal $valid.runtime_id 'windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402' `
    'Valid record must retain the sole fixed runtime ID string'
Assert-Equal $valid.concurrent_sessions 4 'Valid record must retain concurrent session count'
Assert-Equal ($valid.start_to_window_ms -join ',') '11,22,33,44' `
    'Valid record must retain every start-to-window measurement'
Assert-Equal (($valid.per_session | ForEach-Object { $_.index }) -join ',') '1,2,3,4' `
    'Valid record must retain exact session indices'
Assert-Equal (($valid.per_session | ForEach-Object { $_.max_working_set_bytes }) -join ',') `
    '100,100,100,100' 'Valid record must retain per-session working-set maxima'
Assert-Equal (($valid.per_session | ForEach-Object { $_.final_working_set_bytes }) -join ',') `
    '90,90,90,90' 'Valid record must retain per-session final working sets'
Assert-Equal (($valid.per_session | ForEach-Object { $_.max_private_bytes }) -join ',') `
    '200,200,200,200' 'Valid record must retain per-session private-byte maxima'
Assert-Equal (($valid.per_session | ForEach-Object { $_.final_private_bytes }) -join ',') `
    '180,180,180,180' 'Valid record must retain per-session final private bytes'
Assert-Equal (($valid.per_session | ForEach-Object { $_.max_handle_count }) -join ',') `
    '10,10,10,10' 'Valid record must retain per-session handle maxima'
Assert-Equal (($valid.per_session | ForEach-Object { $_.cpu_percent_one_core_x100 }) -join ',') `
    '101,102,103,104' 'Valid record must retain per-session CPU measurements'
Assert-Equal $valid.aggregate.first_working_set_bytes 300 'Valid record must retain first working set'
Assert-Equal $valid.aggregate.max_working_set_bytes 400 'Valid record must retain max working set'
Assert-Equal $valid.aggregate.final_working_set_bytes 350 'Valid record must retain final working set'
Assert-Equal $valid.aggregate.working_set_growth_bytes 50 'Valid record must retain working-set growth'
Assert-Equal $valid.aggregate.first_private_bytes 600 'Valid record must retain first private bytes'
Assert-Equal $valid.aggregate.max_private_bytes 800 'Valid record must retain max private bytes'
Assert-Equal $valid.aggregate.final_private_bytes 700 'Valid record must retain final private bytes'
Assert-Equal $valid.aggregate.private_growth_bytes 100 'Valid record must retain private growth'
Assert-Equal $valid.aggregate.max_handle_count 40 'Valid record must retain aggregate handles'
Assert-Equal $valid.aggregate.cpu_percent_one_core_x100 410 'Valid record must retain aggregate CPU'

    Write-Output 'PASS: Live runtime bridge fail-closed contracts'
}
finally {
    Restore-ZeusLiveEnvironmentState -State $contractOriginalEnvironment
    Assert-LiveEnvironmentStateEqual -Expected $contractOriginalEnvironment `
        -Because 'The complete live contract must preserve its caller environment'
}
