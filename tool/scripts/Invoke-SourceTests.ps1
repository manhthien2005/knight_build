$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = Split-Path -Parent $PSScriptRoot
$powerShell = Join-Path $PSHOME 'pwsh.exe'
$testScripts = @(
    (Join-Path $projectRoot 'tests\RuntimeProvisioning.Tests.ps1'),
    (Join-Path $projectRoot 'tests\TestTiers.Tests.ps1'),
    (Join-Path $projectRoot 'tests\SmokeLauncher.Tests.ps1'),
    (Join-Path $projectRoot 'tests\LiveRuntimeBridge.Tests.ps1'),
    (Join-Path $projectRoot 'tests\WindowsManagerControlLive.Tests.ps1'),
    (Join-Path $projectRoot 'tests\WindowsManagerWorkerLive.Tests.ps1'),
    (Join-Path $projectRoot 'tests\WindowsAccountUi.Tests.ps1'),
    (Join-Path $projectRoot 'tests\WindowsAccountUiLive.Tests.ps1'),
    (Join-Path $projectRoot 'tests\WindowsAccountUiElevatedAcl.Tests.ps1')
)

foreach ($testScript in $testScripts) {
    & $powerShell -NoProfile -File $testScript
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

& $powerShell -NoProfile -File (Join-Path $PSScriptRoot 'Invoke-Cargo.ps1') `
    test --locked --manifest-path (Join-Path $projectRoot 'Cargo.toml') --workspace --all-targets
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

Write-Output 'PASS: Source test tier'
