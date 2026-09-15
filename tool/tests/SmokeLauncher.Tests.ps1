$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$modulePath = Join-Path $PSScriptRoot '..\launcher\Zeus.SmokeLauncher.psm1'
if (-not (Test-Path -LiteralPath $modulePath -PathType Leaf)) {
    throw "RED: launcher module is missing: $modulePath"
}

Import-Module $modulePath -Force

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

function Assert-Contains {
    param(
        [Parameter(Mandatory)] [object[]] $Items,
        [Parameter(Mandatory)] [string] $Expected,
        [Parameter(Mandatory)] [string] $Because
    )

    if ($Items -notcontains $Expected) {
        throw "$Because. Missing '$Expected'."
    }
}

$profileId = '4c5f6da1-3b8a-4d61-bb6e-7bd8a64f0fa2'
$runtimeRoot = 'C:\Zeus Test\runtime'
$dataRoot = 'C:\Zeus Test\data'
$spec = New-ZeusSmokeLaunchSpec -RuntimeRoot $runtimeRoot -DataRoot $dataRoot -ProfileId $profileId

$profileRoot = "C:\Zeus Test\data\profiles\$profileId"
Assert-Equal $spec.JavaExecutable 'C:\Zeus Test\runtime\jre\bin\javaw.exe' 'Launcher must use the bundled Windows Java 11 runtime'
Assert-Equal $spec.WorkingDirectory $profileRoot 'Launcher must isolate the child working directory per profile'
Assert-Equal $spec.Environment.TEMP "$profileRoot\temp" 'TEMP must stay inside the profile root'
Assert-Equal $spec.Environment.TMP "$profileRoot\temp" 'TMP must stay inside the profile root'
Assert-Contains $spec.Arguments "-Duser.home=$profileRoot\microemu-home" 'MicroEmulator home must be profile-local'
Assert-Contains $spec.Arguments "-Djava.io.tmpdir=$profileRoot\temp" 'Java temp must be profile-local'
Assert-Contains $spec.Arguments '-Xms16m' 'Initial heap must avoid a large eager reservation'
Assert-Contains $spec.Arguments '-Xmx128m' 'Smoke runtime must have a bounded heap'
Assert-Contains $spec.Arguments '-XX:+UseSerialGC' 'Small one-process workload must use the low-overhead collector'
Assert-Contains $spec.Arguments '-XX:-UsePerfData' 'JVM must not create hsperfdata with broader ACLs inside the profile temp directory'
Assert-Contains $spec.Arguments '-cp' 'JVM must load the game and emulator on one explicit classpath'
Assert-Contains $spec.Arguments 'C:\Zeus Test\runtime\microemulator\microemulator.jar;C:\Zeus Test\runtime\game\KnightOnline_402.jar' 'Classpath launch must contain only the pinned emulator and game JARs'
Assert-Contains $spec.Arguments 'org.microemu.app.Main' 'Launcher must invoke the MicroEmulator desktop main class'
Assert-Contains $spec.Arguments '--resizableDevice' 'Launcher must explicitly select the resizable device'
Assert-Contains $spec.Arguments '240' 'Launcher must use the reviewed screen width'
Assert-Contains $spec.Arguments '320' 'Launcher must use the reviewed screen height'
Assert-Contains $spec.Arguments '--rms' 'Launcher must enable explicit RMS persistence'
Assert-Contains $spec.Arguments 'file' 'Launcher must use file-backed RMS'
Assert-Contains $spec.Arguments '--id' 'Launcher must pass an isolated emulator ID'
Assert-Contains $spec.Arguments $profileId 'Launcher must derive the emulator ID from the profile UUID'
Assert-Contains $spec.Arguments '--quit' 'Emulator must exit after MIDlet destroy'
Assert-Contains $spec.Arguments 'com.silverknight.TemMidlet' 'Launcher must start the game MIDlet directly instead of stopping at the emulator launcher'
Assert-Equal ($spec.Arguments -contains '-jar') $false 'JAR-location mode must not be used because it stops at the Start screen'

$invalidIdRejected = $false
try {
    New-ZeusSmokeLaunchSpec -RuntimeRoot $runtimeRoot -DataRoot $dataRoot -ProfileId '..\shared' | Out-Null
}
catch {
    $invalidIdRejected = $true
}

Assert-Equal $invalidIdRejected $true 'Launcher must reject path-like profile identifiers'

Write-Output 'PASS: SmokeLauncher contract'
