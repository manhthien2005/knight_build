$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$modulePath = Join-Path $PSScriptRoot '..\scripts\Zeus.TestTiers.psm1'
if (-not (Test-Path -LiteralPath $modulePath -PathType Leaf)) {
    throw "RED: test-tier assertion module is missing: $modulePath"
}

Import-Module $modulePath -Force

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

$testName = 'validates_exact_local_descriptor_as_needs_validation'
Assert-ZeusExactTestListing -Output @("$testName`: test") -ExpectedTestName $testName
Assert-ThrowsLike -Pattern '*exactly one listed test*' `
    -Because 'A removed or renamed exact test must fail before execution' -Action {
        Assert-ZeusExactTestListing -Output @() -ExpectedTestName $testName
    }
Assert-ThrowsLike -Pattern '*exactly one listed test*' `
    -Because 'An unreviewed additional ignored test in the target must fail closed' -Action {
        Assert-ZeusExactTestListing -Output @("$testName`: test", 'unexpected_exact_test: test') `
            -ExpectedTestName $testName
    }

$onePassed = @(
    'running 1 test',
    "test $testName ... ok",
    'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.01s'
)
Assert-ZeusSingleExactTestResult -Output $onePassed -ExpectedTestName $testName
$splitPassed = @(
    'running 1 test',
    "test $testName ... ",
    'ZEUS_LIVE_PERF_V1={"schema_version":1}',
    'ok',
    'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.01s'
)
Assert-ZeusSingleExactTestResult -Output $splitPassed -ExpectedTestName $testName
Assert-ThrowsLike -Pattern '*exactly one passing test result*' `
    -Because 'Split libtest output without one standalone ok must fail closed' -Action {
        Assert-ZeusSingleExactTestResult -Output @($splitPassed | Where-Object { $_ -ne 'ok' }) `
            -ExpectedTestName $testName
    }
Assert-ThrowsLike -Pattern '*exactly one passing test result*' `
    -Because 'Split libtest output with duplicate standalone ok markers is ambiguous' -Action {
        Assert-ZeusSingleExactTestResult -Output @($splitPassed[0..3] + 'ok' + $splitPassed[4]) `
            -ExpectedTestName $testName
    }
Assert-ThrowsLike -Pattern '*exactly one passing test result*' `
    -Because 'A mismatched split execution marker must fail closed' -Action {
        $mismatched = @($splitPassed)
        $mismatched[1] = 'test unexpected_exact_test ... '
        Assert-ZeusSingleExactTestResult -Output $mismatched -ExpectedTestName $testName
    }
Assert-ThrowsLike -Pattern '*exactly one passing test result*' `
    -Because 'An extra test execution marker must fail closed' -Action {
        Assert-ZeusSingleExactTestResult `
            -Output @($splitPassed[0..2] + 'test unexpected_exact_test ... ok' + $splitPassed[3..4]) `
            -ExpectedTestName $testName
    }
Assert-ThrowsLike -Pattern '*exactly one passing test result*' `
    -Because 'An extra non-passing libtest execution marker must fail closed' -Action {
        Assert-ZeusSingleExactTestResult -Output @(
            'running 1 test',
            "test $testName ... ok",
            'test unexpected_exact_test ... FAILED',
            'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.01s'
        ) -ExpectedTestName $testName
    }
Assert-ThrowsLike -Pattern '*exactly one passing test result*' `
    -Because 'Combined and standalone success markers together are ambiguous' -Action {
        Assert-ZeusSingleExactTestResult -Output @(
            'running 1 test',
            "test $testName ... ok",
            'ok',
            'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.01s'
        ) -ExpectedTestName $testName
    }
Assert-ThrowsLike -Pattern '*exactly one passing test result*' `
    -Because 'Cargo exit zero with zero selected tests must not satisfy the exact tier' -Action {
        Assert-ZeusSingleExactTestResult -Output @(
            'running 0 tests',
            'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.00s'
        ) -ExpectedTestName $testName
    }
Assert-ThrowsLike -Pattern '*exactly one passing test result*' `
    -Because 'Multiple test-result summaries must not be accepted as one exact test' -Action {
        Assert-ZeusSingleExactTestResult -Output @($onePassed + $onePassed) `
            -ExpectedTestName $testName
    }

Write-Output 'PASS: Test-tier fail-closed contracts'
