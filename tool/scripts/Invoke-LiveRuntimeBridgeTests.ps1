[CmdletBinding()]
param(
    [string] $RuntimeRoot = (Join-Path $PSScriptRoot '..\runtimes\windows-x64\temurin-11.0.32+9_microemu-2.0.4_ko402'),
    [switch] $AllowGameLaunch
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $AllowGameLaunch) {
    throw 'live game launch requires -AllowGameLaunch'
}

if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne
    [Runtime.InteropServices.Architecture]::X64) {
    throw 'Windows live runtime bridge v1 requires Windows x64.'
}

$projectRoot = (Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot) -ErrorAction Stop).
    ProviderPath
$runtimeItem = Get-Item -LiteralPath $RuntimeRoot -Force -ErrorAction Stop
if (-not $runtimeItem.PSIsContainer) {
    throw 'Windows live runtime bridge v1 requires a runtime directory.'
}
$runtimeFull = (Resolve-Path -LiteralPath $runtimeItem.FullName -ErrorAction Stop).ProviderPath
$runtimeDriveRoot = [IO.Path]::GetPathRoot($runtimeFull)
if ([string]::IsNullOrWhiteSpace($runtimeDriveRoot)) {
    throw 'Windows live runtime bridge v1 requires a fixed local runtime directory.'
}
$runtimeDrive = [IO.DriveInfo]::new($runtimeDriveRoot)
if ($runtimeDrive.DriveType -ne [IO.DriveType]::Fixed) {
    throw 'Windows live runtime bridge v1 requires a fixed local runtime directory.'
}

$provisioner = Join-Path $PSScriptRoot 'Provision-ExactRuntime.ps1'
$preflight = (& $provisioner -RuntimeRoot $runtimeFull -VerifyOnly) | ConvertFrom-Json
if ($preflight.status -ne 'verified' -or
    $preflight.runtime_id -cne 'windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402' -or
    $preflight.jre_file_count -ne 337) {
    throw 'Windows live runtime bridge v1 exact-runtime preflight did not match the pinned tuple.'
}

$powerShell = Join-Path $PSHOME 'pwsh.exe'
$cargoWrapper = Join-Path $PSScriptRoot 'Invoke-Cargo.ps1'
$manifestPath = Join-Path $projectRoot 'Cargo.toml'
Import-Module (Join-Path $PSScriptRoot 'Zeus.TestTiers.psm1') -Force
$liveTests = @(
    'session_supervisor::windows_live_runtime_tests::exact_runtime_launches_through_production_supervisor_and_hard_stops',
    'session_supervisor::windows_live_runtime_tests::four_exact_profiles_stay_isolated_within_performance_bounds',
    'session_supervisor::windows_live_runtime_tests::real_java_exits_when_supervisor_owner_is_terminated'
)
$performanceTest = $liveTests[1]
$performanceRecord = $null
$previousLiveEnvironment = Get-ZeusLiveEnvironmentState
try {
    [Environment]::SetEnvironmentVariable(
        'ZEUS_EXACT_RUNTIME_ROOT',
        $runtimeFull,
        [EnvironmentVariableTarget]::Process
    )
    [Environment]::SetEnvironmentVariable(
        'ZEUS_LIVE_RUNTIME_BRIDGE',
        'windows-live-runtime-bridge-v1-approved',
        [EnvironmentVariableTarget]::Process
    )
    foreach ($testName in $liveTests) {
        $listArguments = @(
            'test', '--locked', '--manifest-path', $manifestPath, '-p', 'zeus-core',
            '--lib', $testName, '--', '--ignored', '--list'
        )
        $listOutput = @(& $powerShell -NoProfile -File $cargoWrapper @listArguments 2>&1)
        $listExitCode = $LASTEXITCODE
        foreach ($line in $listOutput) { Write-Output $line }
        if ($listExitCode -ne 0) {
            exit $listExitCode
        }
        Assert-ZeusExactTestListing -Output $listOutput -ExpectedTestName $testName

        $runArguments = @(
            'test', '--locked', '--manifest-path', $manifestPath, '-p', 'zeus-core',
            '--lib', $testName, '--', '--ignored', '--exact', '--test-threads=1', '--nocapture'
        )
        $runOutput = @(& $powerShell -NoProfile -File $cargoWrapper @runArguments 2>&1)
        $runExitCode = $LASTEXITCODE
        foreach ($line in $runOutput) { Write-Output $line }
        if ($runExitCode -ne 0) {
            exit $runExitCode
        }
        Assert-ZeusSingleExactTestResult -Output $runOutput -ExpectedTestName $testName
        if ($testName -eq $performanceTest) {
            $performanceRecord = Read-ZeusLivePerformanceRecord -Output $runOutput
        }
    }
}
finally {
    Restore-ZeusLiveEnvironmentState -State $previousLiveEnvironment
}

if ($null -eq $performanceRecord) {
    throw 'Windows live runtime bridge v1 produced no performance record.'
}
$performanceRecord | ConvertTo-Json -Depth 8 -Compress
Write-Output 'PASS: Windows live runtime bridge v1'
