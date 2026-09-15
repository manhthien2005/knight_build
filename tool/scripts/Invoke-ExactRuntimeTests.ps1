[CmdletBinding()]
param(
    [string] $RuntimeRoot = (Join-Path $PSScriptRoot '..\runtimes\windows-x64\temurin-11.0.32+9_microemu-2.0.4_ko402'),
    [string] $CacheRoot = (Join-Path $PSScriptRoot '..\.source-cache\runtime-setup')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = Split-Path -Parent $PSScriptRoot
$powerShell = Join-Path $PSHOME 'pwsh.exe'
$provisioner = Join-Path $PSScriptRoot 'Provision-ExactRuntime.ps1'
$runtimeFull = [IO.Path]::GetFullPath($RuntimeRoot)
$preflight = (& $provisioner -RuntimeRoot $runtimeFull -CacheRoot $CacheRoot -VerifyOnly) |
    ConvertFrom-Json
if ($preflight.status -ne 'verified') {
    throw "Exact-runtime preflight returned an unexpected status: $($preflight.status)"
}

$cargoWrapper = Join-Path $PSScriptRoot 'Invoke-Cargo.ps1'
$manifestPath = Join-Path $projectRoot 'Cargo.toml'
$testTierModule = Join-Path $PSScriptRoot 'Zeus.TestTiers.psm1'
Import-Module $testTierModule -Force
$exactTests = @(
    [pscustomobject]@{
        target = 'runtime_validation'
        name = 'validates_exact_local_descriptor_as_needs_validation'
    },
    [pscustomobject]@{
        target = 'launch_snapshot'
        name = 'exact_windows_runtime_produces_a_needs_validation_snapshot'
    }
)

$previousRuntimeRoot = [Environment]::GetEnvironmentVariable(
    'ZEUS_EXACT_RUNTIME_ROOT',
    [EnvironmentVariableTarget]::Process
)
try {
    [Environment]::SetEnvironmentVariable(
        'ZEUS_EXACT_RUNTIME_ROOT',
        $runtimeFull,
        [EnvironmentVariableTarget]::Process
    )
    foreach ($exactTest in $exactTests) {
        $listArguments = @(
            'test', '--locked', '--manifest-path', $manifestPath, '-p', 'zeus-core',
            '--test', $exactTest.target, '--', '--ignored', '--list'
        )
        $listOutput = @(& $powerShell -NoProfile -File $cargoWrapper @listArguments 2>&1)
        $listExitCode = $LASTEXITCODE
        foreach ($line in $listOutput) { Write-Output $line }
        if ($listExitCode -ne 0) {
            exit $listExitCode
        }
        Assert-ZeusExactTestListing -Output $listOutput -ExpectedTestName $exactTest.name

        $runArguments = @(
            'test', '--locked', '--manifest-path', $manifestPath, '-p', 'zeus-core',
            '--test', $exactTest.target, $exactTest.name, '--', '--ignored', '--exact',
            '--test-threads=1', '--nocapture'
        )
        $runOutput = @(& $powerShell -NoProfile -File $cargoWrapper @runArguments 2>&1)
        $runExitCode = $LASTEXITCODE
        foreach ($line in $runOutput) { Write-Output $line }
        if ($runExitCode -ne 0) {
            exit $runExitCode
        }
        Assert-ZeusSingleExactTestResult -Output $runOutput -ExpectedTestName $exactTest.name
    }

    & $powerShell -NoProfile -File `
        (Join-Path $projectRoot 'tests\SmokeLauncher.ExactRuntime.Tests.ps1') `
        -RuntimeRoot $runtimeFull
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}
finally {
    [Environment]::SetEnvironmentVariable(
        'ZEUS_EXACT_RUNTIME_ROOT',
        $previousRuntimeRoot,
        [EnvironmentVariableTarget]::Process
    )
}

Write-Output 'PASS: Exact-runtime test tier'
