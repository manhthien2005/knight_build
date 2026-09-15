Set-StrictMode -Version Latest

function Assert-ZeusExactTestListing {
    param(
        [Parameter(Mandatory)] [AllowEmptyCollection()] [object[]] $Output,
        [Parameter(Mandatory)] [string] $ExpectedTestName
    )

    $listed = @($Output | ForEach-Object { [string] $_ } | Where-Object {
        $_ -match ': test$'
    })
    $expected = "$ExpectedTestName`: test"
    if ($listed.Count -ne 1 -or $listed[0] -ne $expected) {
        throw "Exact-runtime target must expose exactly one listed test '$expected'; found: $($listed -join ', ')"
    }
}

function Assert-ZeusSingleExactTestResult {
    param(
        [Parameter(Mandatory)] [AllowEmptyCollection()] [object[]] $Output,
        [Parameter(Mandatory)] [string] $ExpectedTestName
    )

    $lines = @($Output | ForEach-Object { [string] $_ })
    $summaries = @($lines | Where-Object { $_ -match '^test result:' })
    $expectedSummary = '^test result: ok\. 1 passed; 0 failed; 0 ignored; [0-9]+ measured; [0-9]+ filtered out; finished in .+s$'
    $runningMarkers = @($lines | Where-Object { $_ -ceq 'running 1 test' })
    $expectedCombinedExecution = "test $ExpectedTestName ... ok"
    $expectedSplitExecution = "test $ExpectedTestName ..."
    $executionMarkers = @($lines | Where-Object {
        $_ -match '^test .+ \.\.\.'
    })
    $combinedMarkers = @($executionMarkers | Where-Object {
        $_ -ceq $expectedCombinedExecution
    })
    $splitMarkers = @($executionMarkers | Where-Object {
        $_.TrimEnd() -ceq $expectedSplitExecution
    })
    $standaloneOkMarkers = @($lines | Where-Object { $_ -ceq 'ok' })
    $combinedIsExact = $executionMarkers.Count -eq 1 -and
        $combinedMarkers.Count -eq 1 -and $splitMarkers.Count -eq 0 -and
        $standaloneOkMarkers.Count -eq 0
    $splitIsExact = $executionMarkers.Count -eq 1 -and
        $combinedMarkers.Count -eq 0 -and $splitMarkers.Count -eq 1 -and
        $standaloneOkMarkers.Count -eq 1
    if ($summaries.Count -ne 1 -or $summaries[0] -notmatch $expectedSummary -or
        $runningMarkers.Count -ne 1 -or ($combinedIsExact -eq $splitIsExact)) {
        throw "Exact-runtime Cargo invocation must produce exactly one passing test result for '$ExpectedTestName'."
    }
}

function Get-ZeusLiveEnvironmentState {
    $variables = [Environment]::GetEnvironmentVariables(
        [EnvironmentVariableTarget]::Process
    )
    [pscustomobject] [ordered] @{
        runtime_root_present = $variables.Contains('ZEUS_EXACT_RUNTIME_ROOT')
        runtime_root_value = [Environment]::GetEnvironmentVariable(
            'ZEUS_EXACT_RUNTIME_ROOT',
            [EnvironmentVariableTarget]::Process
        )
        live_approval_present = $variables.Contains('ZEUS_LIVE_RUNTIME_BRIDGE')
        live_approval_value = [Environment]::GetEnvironmentVariable(
            'ZEUS_LIVE_RUNTIME_BRIDGE',
            [EnvironmentVariableTarget]::Process
        )
    }
}

function Restore-ZeusLiveEnvironmentState {
    param([Parameter(Mandatory)] [pscustomobject] $State)

    $expectedProperties = @(
        'runtime_root_present',
        'runtime_root_value',
        'live_approval_present',
        'live_approval_value'
    )
    $actualProperties = @($State.PSObject.Properties.Name)
    $propertyDifference = @(
        Compare-Object -ReferenceObject $expectedProperties -DifferenceObject $actualProperties `
            -CaseSensitive
    )
    if ($actualProperties.Count -ne $expectedProperties.Count -or
        $propertyDifference.Count -ne 0 -or
        $State.runtime_root_present -isnot [bool] -or
        $State.live_approval_present -isnot [bool] -or
        ($State.runtime_root_present -and $State.runtime_root_value -isnot [string]) -or
        (-not $State.runtime_root_present -and $null -ne $State.runtime_root_value) -or
        ($State.live_approval_present -and $State.live_approval_value -isnot [string]) -or
        (-not $State.live_approval_present -and $null -ne $State.live_approval_value)) {
        throw 'Invalid live environment state.'
    }

    if ($State.runtime_root_present) {
        [Environment]::SetEnvironmentVariable(
            'ZEUS_EXACT_RUNTIME_ROOT',
            $State.runtime_root_value,
            [EnvironmentVariableTarget]::Process
        )
    }
    else {
        Remove-Item -LiteralPath 'Env:ZEUS_EXACT_RUNTIME_ROOT' -Force -ErrorAction SilentlyContinue
    }

    if ($State.live_approval_present) {
        [Environment]::SetEnvironmentVariable(
            'ZEUS_LIVE_RUNTIME_BRIDGE',
            $State.live_approval_value,
            [EnvironmentVariableTarget]::Process
        )
    }
    else {
        Remove-Item -LiteralPath 'Env:ZEUS_LIVE_RUNTIME_BRIDGE' -Force -ErrorAction SilentlyContinue
    }
}

function Assert-ZeusLiveRecordProperties {
    param(
        [Parameter(Mandatory)] [object] $Value,
        [Parameter(Mandatory)] [string[]] $Expected,
        [Parameter(Mandatory)] [string] $Label
    )

    if ($Value -isnot [pscustomobject]) {
        throw "Invalid live performance record: $Label must be an object."
    }
    $actual = @($Value.PSObject.Properties.Name)
    $difference = @(
        Compare-Object -ReferenceObject $Expected -DifferenceObject $actual -CaseSensitive
    )
    if ($actual.Count -ne $Expected.Count -or $difference.Count -ne 0) {
        throw "Invalid live performance record: $Label property set is not exact."
    }
}

function Assert-ZeusLiveRecordNumber {
    param(
        [Parameter(Mandatory)] [object] $Value,
        [Parameter(Mandatory)] [string] $Label,
        [switch] $AllowSigned
    )

    $isIntegralType = $Value -is [byte] -or $Value -is [sbyte] -or
        $Value -is [int16] -or $Value -is [uint16] -or
        $Value -is [int32] -or $Value -is [uint32] -or
        $Value -is [int64] -or $Value -is [uint64] -or
        $Value -is [Numerics.BigInteger]
    $isFloatingType = $Value -is [single] -or $Value -is [double]
    if (-not $isIntegralType -and -not $isFloatingType -and $Value -isnot [decimal]) {
        throw "Invalid live performance record: $Label must be numeric."
    }

    if ($isFloatingType) {
        $floating = [double] $Value
        if ([double]::IsNaN($floating) -or [double]::IsInfinity($floating) -or
            [Math]::Truncate($floating) -ne $floating) {
            throw "Invalid live performance record: $Label must be an integer."
        }
    }
    elseif ($Value -is [decimal] -and [decimal]::Truncate($Value) -ne $Value) {
        throw "Invalid live performance record: $Label must be an integer."
    }

    $integer = [Numerics.BigInteger] $Value
    if ($AllowSigned) {
        $minimum = [Numerics.BigInteger]::Parse('-9223372036854775808')
        $maximum = [Numerics.BigInteger]::Parse('9223372036854775807')
    }
    else {
        $minimum = [Numerics.BigInteger]::Zero
        $maximum = [Numerics.BigInteger]::Parse('18446744073709551615')
    }
    if ($integer -lt $minimum -or $integer -gt $maximum) {
        throw "Invalid live performance record: $Label is outside its integer domain."
    }
}

function Assert-ZeusLiveRecordBoolean {
    param(
        [Parameter(Mandatory)] [object] $Value,
        [Parameter(Mandatory)] [bool] $Expected,
        [Parameter(Mandatory)] [string] $Label
    )

    if ($Value -isnot [bool] -or $Value -ne $Expected) {
        throw "Invalid live performance record: $Label has the wrong fixed value."
    }
}

function Read-ZeusLivePerformanceRecord {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [object[]] $Output
    )

    $prefix = 'ZEUS_LIVE_PERF_V1='
    $matching = @($Output | ForEach-Object { [string] $_ } | Where-Object {
        $_.StartsWith($prefix, [StringComparison]::Ordinal)
    })
    if ($matching.Count -ne 1) {
        throw 'Invalid live performance record: exactly one prefixed line is required.'
    }
    $json = $matching[0].Substring($prefix.Length)
    if ($json.Length -lt 1 -or $json.Length -gt 65536) {
        throw 'Invalid live performance record: JSON length is outside 1..65536.'
    }
    try {
        $record = $json | ConvertFrom-Json -ErrorAction Stop
    }
    catch {
        throw 'Invalid live performance record: JSON parsing failed.'
    }

    $topLevelProperties = @(
        'schema_version',
        'runtime_id',
        'concurrent_sessions',
        'stabilization_seconds',
        'observation_seconds',
        'sample_interval_seconds',
        'samples_per_session',
        'representative_of_1gib_target',
        'capacity_rejection_confirmed',
        'all_windows_responsive',
        'cleanup_confirmed',
        'start_to_window_ms',
        'per_session',
        'aggregate'
    )
    Assert-ZeusLiveRecordProperties -Value $record -Expected $topLevelProperties -Label 'top-level'

    $fixedNumbers = [ordered] @{
        schema_version = 1
        concurrent_sessions = 4
        stabilization_seconds = 15
        observation_seconds = 60
        sample_interval_seconds = 5
        samples_per_session = 13
    }
    foreach ($property in $fixedNumbers.GetEnumerator()) {
        $value = $record.($property.Key)
        Assert-ZeusLiveRecordNumber -Value $value -Label $property.Key
        if ($value -ne $property.Value) {
            throw "Invalid live performance record: $($property.Key) has the wrong fixed value."
        }
    }
    if ($record.runtime_id -isnot [string] -or $record.runtime_id -cne
        'windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402') {
        throw 'Invalid live performance record: runtime_id has the wrong fixed value.'
    }
    Assert-ZeusLiveRecordBoolean -Value $record.representative_of_1gib_target -Expected $false `
        -Label 'representative_of_1gib_target'
    Assert-ZeusLiveRecordBoolean -Value $record.capacity_rejection_confirmed -Expected $true `
        -Label 'capacity_rejection_confirmed'
    Assert-ZeusLiveRecordBoolean -Value $record.all_windows_responsive -Expected $true `
        -Label 'all_windows_responsive'
    Assert-ZeusLiveRecordBoolean -Value $record.cleanup_confirmed -Expected $true `
        -Label 'cleanup_confirmed'

    $startMeasurements = @($record.start_to_window_ms)
    if ($startMeasurements.Count -ne 4) {
        throw 'Invalid live performance record: start_to_window_ms must contain four values.'
    }
    foreach ($measurement in $startMeasurements) {
        Assert-ZeusLiveRecordNumber -Value $measurement -Label 'start_to_window_ms value'
    }

    $perSessionProperties = @(
        'index',
        'max_working_set_bytes',
        'final_working_set_bytes',
        'max_private_bytes',
        'final_private_bytes',
        'max_handle_count',
        'cpu_percent_one_core_x100'
    )
    $sessions = @($record.per_session)
    if ($sessions.Count -ne 4) {
        throw 'Invalid live performance record: per_session must contain four entries.'
    }
    $workingSetMaximumSum = 0.0
    $privateMaximumSum = 0.0
    $handleMaximumSum = 0.0
    for ($offset = 0; $offset -lt $sessions.Count; $offset++) {
        $session = $sessions[$offset]
        Assert-ZeusLiveRecordProperties -Value $session -Expected $perSessionProperties `
            -Label "per_session[$offset]"
        foreach ($property in $perSessionProperties) {
            Assert-ZeusLiveRecordNumber -Value $session.$property `
                -Label "per_session[$offset].$property"
        }
        if ($session.index -ne ($offset + 1)) {
            throw 'Invalid live performance record: per_session indices must be exactly 1,2,3,4.'
        }
        if ($session.max_working_set_bytes -lt $session.final_working_set_bytes -or
            $session.max_private_bytes -lt $session.final_private_bytes) {
            throw "Invalid live performance record: per_session[$offset] maximum is below final."
        }
        if ($session.max_working_set_bytes -gt 268435456 -or
            $session.max_private_bytes -gt 268435456 -or
            $session.max_handle_count -gt 1200) {
            throw "Invalid live performance record: per_session[$offset] exceeds a ceiling."
        }
        $workingSetMaximumSum += [double] $session.max_working_set_bytes
        $privateMaximumSum += [double] $session.max_private_bytes
        $handleMaximumSum += [double] $session.max_handle_count
    }

    $aggregateProperties = @(
        'first_working_set_bytes',
        'max_working_set_bytes',
        'final_working_set_bytes',
        'working_set_growth_bytes',
        'first_private_bytes',
        'max_private_bytes',
        'final_private_bytes',
        'private_growth_bytes',
        'max_handle_count',
        'cpu_percent_one_core_x100'
    )
    $aggregate = $record.aggregate
    Assert-ZeusLiveRecordProperties -Value $aggregate -Expected $aggregateProperties -Label 'aggregate'
    foreach ($property in $aggregateProperties) {
        Assert-ZeusLiveRecordNumber -Value $aggregate.$property -Label "aggregate.$property" `
            -AllowSigned:($property -in @('working_set_growth_bytes', 'private_growth_bytes'))
    }
    if ($aggregate.max_working_set_bytes -lt $aggregate.first_working_set_bytes -or
        $aggregate.max_working_set_bytes -lt $aggregate.final_working_set_bytes -or
        $aggregate.max_private_bytes -lt $aggregate.first_private_bytes -or
        $aggregate.max_private_bytes -lt $aggregate.final_private_bytes) {
        throw 'Invalid live performance record: an aggregate maximum is below first or final.'
    }
    if ($aggregate.working_set_growth_bytes -ne
        ($aggregate.final_working_set_bytes - $aggregate.first_working_set_bytes) -or
        $aggregate.private_growth_bytes -ne
        ($aggregate.final_private_bytes - $aggregate.first_private_bytes)) {
        throw 'Invalid live performance record: aggregate growth does not equal signed final minus first.'
    }
    if ($aggregate.max_working_set_bytes -gt 805306368 -or
        $aggregate.max_private_bytes -gt 805306368 -or
        $aggregate.max_handle_count -gt 4800 -or
        $aggregate.cpu_percent_one_core_x100 -gt 10000 -or
        $aggregate.working_set_growth_bytes -gt 134217728 -or
        $aggregate.private_growth_bytes -gt 134217728) {
        throw 'Invalid live performance record: aggregate evidence exceeds a ceiling.'
    }
    if ($aggregate.max_working_set_bytes -gt $workingSetMaximumSum -or
        $aggregate.max_private_bytes -gt $privateMaximumSum -or
        $aggregate.max_handle_count -gt $handleMaximumSum) {
        throw 'Invalid live performance record: aggregate maximum exceeds the checked session sum.'
    }

    $record
}

Export-ModuleMember -Function Assert-ZeusExactTestListing, Assert-ZeusSingleExactTestResult, `
    Get-ZeusLiveEnvironmentState, Restore-ZeusLiveEnvironmentState, `
    Read-ZeusLivePerformanceRecord
