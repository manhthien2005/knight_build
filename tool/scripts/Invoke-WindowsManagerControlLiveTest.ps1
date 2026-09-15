[CmdletBinding()]
param(
    [string] $RuntimeRoot = (Join-Path $PSScriptRoot '..\runtimes\windows-x64\temurin-11.0.32+9_microemu-2.0.4_ko402'),
    [switch] $AllowGameLaunch
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $AllowGameLaunch) {
    throw 'manager live game launch requires -AllowGameLaunch'
}

if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne
    [Runtime.InteropServices.Architecture]::X64) {
    throw 'Windows manager control live test requires Windows x64.'
}

$projectRoot = (Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot) -ErrorAction Stop).
    ProviderPath
$runtimeItem = Get-Item -LiteralPath $RuntimeRoot -Force -ErrorAction Stop
if (-not $runtimeItem.PSIsContainer) {
    throw 'Windows manager control live test requires a runtime directory.'
}
$runtimeFull = (Resolve-Path -LiteralPath $runtimeItem.FullName -ErrorAction Stop).ProviderPath
$runtimeDriveRoot = [IO.Path]::GetPathRoot($runtimeFull)
if ([string]::IsNullOrWhiteSpace($runtimeDriveRoot)) {
    throw 'Windows manager control live test requires a fixed local runtime directory.'
}
$runtimeDrive = [IO.DriveInfo]::new($runtimeDriveRoot)
if ($runtimeDrive.DriveType -ne [IO.DriveType]::Fixed) {
    throw 'Windows manager control live test requires a fixed local runtime directory.'
}

$provisioner = Join-Path $PSScriptRoot 'Provision-ExactRuntime.ps1'
$preflight = (& $provisioner -RuntimeRoot $runtimeFull -VerifyOnly) | ConvertFrom-Json
if ($preflight.status -ne 'verified' -or
    $preflight.runtime_id -cne 'windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402' -or
    $preflight.jre_file_count -ne 337) {
    throw 'Windows manager control live exact-runtime preflight did not match the pinned tuple.'
}

$powerShell = Join-Path $PSHOME 'pwsh.exe'
$cargoWrapper = Join-Path $PSScriptRoot 'Invoke-Cargo.ps1'
$manifestPath = Join-Path $projectRoot 'Cargo.toml'
Import-Module (Join-Path $PSScriptRoot 'Zeus.TestTiers.psm1') -Force
$liveTest = 'manager::windows_live_runtime_tests::exact_runtime_runs_through_public_manager_control'
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

    $listArguments = @(
        'test', '--locked', '--manifest-path', $manifestPath, '-p', 'zeus-core',
        '--lib', $liveTest, '--', '--ignored', '--list'
    )
    $listOutput = @(& $powerShell -NoProfile -File $cargoWrapper @listArguments 2>&1)
    $listExitCode = $LASTEXITCODE
    foreach ($line in $listOutput) { Write-Output $line }
    if ($listExitCode -ne 0) {
        exit $listExitCode
    }
    Assert-ZeusExactTestListing -Output $listOutput -ExpectedTestName $liveTest

    $runArguments = @(
        'test', '--locked', '--manifest-path', $manifestPath, '-p', 'zeus-core',
        '--lib', $liveTest, '--', '--ignored', '--exact', '--test-threads=1', '--nocapture'
    )
    $runOutput = @(& $powerShell -NoProfile -File $cargoWrapper @runArguments 2>&1)
    $runExitCode = $LASTEXITCODE
    foreach ($line in $runOutput) { Write-Output $line }
    if ($runExitCode -ne 0) {
        exit $runExitCode
    }
    Assert-ZeusSingleExactTestResult -Output $runOutput -ExpectedTestName $liveTest
}
finally {
    Restore-ZeusLiveEnvironmentState -State $previousLiveEnvironment
}

Write-Output 'PASS: Windows manager control live test'
