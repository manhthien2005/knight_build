[CmdletBinding()]
param(
    [string] $RuntimeRoot = (Join-Path $PSScriptRoot '..\runtimes\windows-x64\temurin-11.0.32+9_microemu-2.0.4_ko402'),
    [string] $DataRoot = (Join-Path $PSScriptRoot '..\smoke-data'),
    [string] $ProfileId = '4c5f6da1-3b8a-4d61-bb6e-7bd8a64f0fa2',
    [switch] $DryRun,
    [switch] $Wait
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot 'Zeus.SmokeLauncher.psm1') -Force

$spec = New-ZeusSmokeLaunchSpec -RuntimeRoot $RuntimeRoot -DataRoot $DataRoot -ProfileId $ProfileId
$jreRelease = Join-Path ([IO.Path]::GetFullPath($RuntimeRoot)) 'jre\release'
$requiredFiles = @(
    $spec.JavaExecutable
    $spec.JavaConsoleExecutable
    $jreRelease
    $spec.MicroemulatorJar
    $spec.GameJar
)

foreach ($requiredFile in $requiredFiles) {
    if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) {
        throw "Runtime file is missing: $requiredFile"
    }
}

$microemulatorSha256 = (Get-FileHash -LiteralPath $spec.MicroemulatorJar -Algorithm SHA256).Hash.ToLowerInvariant()
if ($microemulatorSha256 -ne 'dbd5f3eb8365d3e839d6a203149e0e3776fc1a0585e16ac1fc23f76c9fcae1c6') {
    throw "MicroEmulator checksum mismatch: $microemulatorSha256"
}

$gameSha256 = (Get-FileHash -LiteralPath $spec.GameJar -Algorithm SHA256).Hash.ToLowerInvariant()
if ($gameSha256 -ne '6608bb0c77f03749e46165f711e9566dca4e172ce232256497b35faafe74c259') {
    throw "Game checksum mismatch: $gameSha256"
}

$jreReleaseSha256 = (Get-FileHash -LiteralPath $jreRelease -Algorithm SHA256).Hash.ToLowerInvariant()
if ($jreReleaseSha256 -cne '42f4b610e7b8976fbef0b9e2217e5c7b40b9475e0cd500e8be2e90aa3742705f') {
    throw "JRE release checksum mismatch: $jreReleaseSha256"
}

$validation = [ordered]@{
    RuntimeValid = $true
    JavaVersion = '11.0.32+9'
    MicroemulatorSha256 = $microemulatorSha256
    GameSha256 = $gameSha256
    LaunchSpec = $spec
}

if ($DryRun) {
    $validation | ConvertTo-Json -Depth 8
    exit 0
}

$javaVersionOutput = (& $spec.JavaConsoleExecutable -version 2>&1) -join [Environment]::NewLine
if ($LASTEXITCODE -ne 0 -or $javaVersionOutput -notmatch 'Temurin-11\.0\.32\+9') {
    throw "Bundled Java is not the pinned Temurin 11 runtime: $javaVersionOutput"
}

$dataRootFull = [IO.Path]::GetFullPath($DataRoot)
$profileRootFull = [IO.Path]::GetFullPath($spec.ProfileRoot)
if (-not $profileRootFull.StartsWith($dataRootFull + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Profile root escaped the configured data root."
}

foreach ($directory in @($spec.ProfileRoot, $spec.MicroemuHome, $spec.TempRoot, $spec.DiagnosticRoot)) {
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $item = Get-Item -LiteralPath $directory -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Profile directory cannot be a reparse point: $directory"
    }
}

$startInfo = [Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $spec.JavaExecutable
$startInfo.WorkingDirectory = $spec.WorkingDirectory
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
foreach ($argument in $spec.Arguments) {
    $startInfo.ArgumentList.Add([string] $argument)
}
foreach ($entry in $spec.Environment.GetEnumerator()) {
    $startInfo.Environment[[string] $entry.Key] = [string] $entry.Value
}

$process = [Diagnostics.Process]::new()
$process.StartInfo = $startInfo
if (-not $process.Start()) {
    throw 'Failed to start the bundled Java runtime.'
}

$launchResult = [ordered]@{
    RuntimeValid = $true
    ProfileId = $spec.ProfileId
    ProcessId = $process.Id
    StartedAtUtc = $process.StartTime.ToUniversalTime().ToString('O')
    ProfileRoot = $spec.ProfileRoot
}

if ($Wait) {
    $process.WaitForExit()
    $launchResult.ExitCode = $process.ExitCode
}

$process.Dispose()
$launchResult | ConvertTo-Json -Depth 4
