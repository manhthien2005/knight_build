Set-StrictMode -Version Latest

function New-ZeusSmokeLaunchSpec {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string] $RuntimeRoot,

        [Parameter(Mandatory)]
        [string] $DataRoot,

        [Parameter(Mandatory)]
        [string] $ProfileId
    )

    $parsedProfileId = [Guid]::Empty
    if (-not [Guid]::TryParseExact($ProfileId, 'D', [ref] $parsedProfileId)) {
        throw "ProfileId must be a UUID in D format."
    }

    $normalizedProfileId = $parsedProfileId.ToString('D')
    $runtimeRootFull = [IO.Path]::GetFullPath($RuntimeRoot)
    $dataRootFull = [IO.Path]::GetFullPath($DataRoot)
    $profileRoot = Join-Path $dataRootFull "profiles\$normalizedProfileId"
    $microemuHome = Join-Path $profileRoot 'microemu-home'
    $tempRoot = Join-Path $profileRoot 'temp'
    $diagnosticRoot = Join-Path $profileRoot 'diagnostic-logs'
    $javaExecutable = Join-Path $runtimeRootFull 'jre\bin\javaw.exe'
    $javaConsoleExecutable = Join-Path $runtimeRootFull 'jre\bin\java.exe'
    $microemulatorJar = Join-Path $runtimeRootFull 'microemulator\microemulator.jar'
    $gameJar = Join-Path $runtimeRootFull 'game\KnightOnline_402.jar'
    $classPath = "$microemulatorJar$([IO.Path]::PathSeparator)$gameJar"

    $arguments = @(
        "-Duser.home=$microemuHome"
        "-Djava.io.tmpdir=$tempRoot"
        '-Xms16m'
        '-Xmx128m'
        '-XX:+UseSerialGC'
        '-XX:-UsePerfData'
        "-XX:ErrorFile=$(Join-Path $diagnosticRoot 'hs_err_pid%p.log')"
        '-cp'
        $classPath
        'org.microemu.app.Main'
        '--resizableDevice'
        '240'
        '320'
        '--rms'
        'file'
        '--id'
        $normalizedProfileId
        '--quit'
        'com.silverknight.TemMidlet'
    )

    [pscustomobject]@{
        ProfileId = $normalizedProfileId
        ProfileRoot = $profileRoot
        MicroemuHome = $microemuHome
        TempRoot = $tempRoot
        DiagnosticRoot = $diagnosticRoot
        JavaExecutable = $javaExecutable
        JavaConsoleExecutable = $javaConsoleExecutable
        MicroemulatorJar = $microemulatorJar
        GameJar = $gameJar
        WorkingDirectory = $profileRoot
        Environment = @{
            TEMP = $tempRoot
            TMP = $tempRoot
            TMPDIR = $tempRoot
        }
        Arguments = $arguments
    }
}

Export-ModuleMember -Function New-ZeusSmokeLaunchSpec
