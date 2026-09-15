#Requires -Version 7.0
<#
.SYNOPSIS
Package contract for the portable Zeus account manager.

.DESCRIPTION
Verifies the assembled folder shape, that the executable is a GUI-subsystem Windows binary with no
non-system UI dependency, and that the real binary starts against fresh state, creates its portable
data root exe-relative, and survives being moved to a different path.

No game is launched. Every temporary artifact is removed in `finally`.
#>
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = Split-Path -Parent $PSScriptRoot
$assembleScript = Join-Path $projectRoot 'scripts\Assemble-WindowsAccountUiPortable.ps1'
$pinnedRuntimeDirectory = 'temurin-11.0.32+9_microemu-2.0.4_ko402'

if (-not (Test-Path -LiteralPath $assembleScript -PathType Leaf)) {
    throw "RED: assembly script is missing: $assembleScript"
}

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

function Assert-True {
    param(
        [Parameter(Mandatory)] [bool] $Condition,
        [Parameter(Mandatory)] [string] $Because
    )

    if (-not $Condition) {
        throw $Because
    }
}

<#
Reads the PE subsystem so a console window can never appear for the operator.
#>
function Get-PeSubsystem {
    param([Parameter(Mandatory)] [string] $Path)

    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $peOffset = [System.BitConverter]::ToInt32($bytes, 0x3c)
    # Subsystem sits at COFF header + 0x5c in the optional header.
    return [System.BitConverter]::ToUInt16($bytes, $peOffset + 0x5c)
}

<#
Launches the assembled binary, waits for its native window, then closes it politely.
#>
function Invoke-AssembledUi {
    param(
        [Parameter(Mandatory)] [string] $Root,
        [int] $TimeoutSeconds = 20
    )

    $executable = Join-Path $Root 'zeus-ui.exe'
    $process = Start-Process -FilePath $executable -PassThru
    try {
        $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
        $title = ''
        while ((Get-Date) -lt $deadline) {
            if ($process.HasExited) {
                throw "UI exited early with code $($process.ExitCode)."
            }
            $process.Refresh()
            if ($process.MainWindowTitle) {
                $title = $process.MainWindowTitle
                break
            }
            Start-Sleep -Milliseconds 200
        }
        if (-not $title) {
            throw 'UI never presented its native window.'
        }
        $null = $process.CloseMainWindow()
        if (-not $process.WaitForExit(10000)) {
            throw 'UI did not exit after WM_CLOSE.'
        }
        return [pscustomobject]@{
            Title    = $title
            ExitCode = $process.ExitCode
        }
    }
    finally {
        if (-not $process.HasExited) {
            $process.Kill()
            $null = $process.WaitForExit(5000)
        }
    }
}

# A path with spaces proves nothing depends on unquoted path handling.
$workspace = Join-Path ([System.IO.Path]::GetTempPath()) ("zeus account ui " + [guid]::NewGuid())
$packageRoot = Join-Path $workspace 'Zeus package'
$movedRoot = Join-Path $workspace 'Zeus moved copy'

try {
    $inventory = & $assembleScript -OutputRoot $packageRoot

    # --- Folder shape -------------------------------------------------------------------------
    Assert-True (Test-Path -LiteralPath (Join-Path $packageRoot 'zeus-ui.exe') -PathType Leaf) `
        'Assembly must place zeus-ui.exe at the package root.'
    # `data/` must be absent until the application creates it with a protected DACL.
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $packageRoot 'data'))) `
        'Assembly must not pre-create the data directory.'
    Assert-True (Test-Path -LiteralPath (Join-Path $packageRoot "runtimes\windows-x64\$pinnedRuntimeDirectory") -PathType Container) `
        'Assembly must create the pinned runtime directory.'
    Assert-Equal $inventory.Executable 'zeus-ui.exe' 'Inventory must name the executable.'

    # --- Copied runtime security --------------------------------------------------------------
    # A plain copy inherits the destination's ACLs, which include Authenticated Users. The
    # application rejects such a runtime as insecure and boots into `Tool chua san sang`, so the
    # assembler must re-protect a supplied runtime with the owner-only allowlist.
    # A runtime-shaped source tree is enough: the assembler copies and re-protects whatever it is
    # given, so this needs no provisioned JRE. It lives outside $workspace because the later
    # stray-directory assertion requires $workspace to hold the package root alone.
    $runtimeWorkspace = Join-Path ([System.IO.Path]::GetTempPath()) ("zeus runtime acl " + [guid]::NewGuid())
    $sourceRuntime = Join-Path $runtimeWorkspace 'source runtime'
    $null = New-Item -ItemType Directory -Path (Join-Path $sourceRuntime 'jre\bin') -Force
    Set-Content -LiteralPath (Join-Path $sourceRuntime 'runtime-descriptor.json') -Value '{}'
    Set-Content -LiteralPath (Join-Path $sourceRuntime 'jre\bin\java.exe') -Value 'stub'
    $securedRoot = Join-Path $runtimeWorkspace 'Zeus secured runtime'
    $null = & $assembleScript -OutputRoot $securedRoot -RuntimeSource $sourceRuntime
    $securedRuntime = Join-Path $securedRoot "runtimes\windows-x64\$pinnedRuntimeDirectory"
    $currentSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $probes = @(
        $securedRuntime,
        (Join-Path $securedRuntime 'runtime-descriptor.json'),
        (Join-Path $securedRuntime 'jre\bin'),
        (Join-Path $securedRuntime 'jre\bin\java.exe')
    )
    foreach ($probe in $probes) {
        $access = @((Get-Acl -LiteralPath $probe).Access)
        Assert-Equal $access.Count 2 `
            "A packaged runtime entry must carry exactly two ACEs: $probe"
        foreach ($entry in $access) {
            Assert-True (-not $entry.IsInherited) `
                "A packaged runtime entry must not inherit ACLs: $probe"
            $sid = $entry.IdentityReference.Translate(
                [Security.Principal.SecurityIdentifier]
            ).Value
            Assert-True ($sid -eq $currentSid -or $sid -eq 'S-1-5-18') `
                "A packaged runtime entry must grant only the current user and SYSTEM: $probe"
        }
    }
    Remove-Item -LiteralPath $runtimeWorkspace -Recurse -Force

    # Exactly one executable ships, and no installer or uninstaller is produced.
    $executables = @(Get-ChildItem -LiteralPath $packageRoot -Recurse -File -Filter '*.exe')
    Assert-Equal $executables.Count 1 'The package must contain exactly one executable.'
    foreach ($forbidden in @('*.msi', '*.reg', 'unins*', '*setup*', '*install*')) {
        $found = @(Get-ChildItem -LiteralPath $packageRoot -Recurse -File -Filter $forbidden)
        Assert-Equal $found.Count 0 "The package must not contain $forbidden."
    }

    # --- Binary contract ---------------------------------------------------------------------
    $executablePath = Join-Path $packageRoot 'zeus-ui.exe'
    # IMAGE_SUBSYSTEM_WINDOWS_GUI = 2. A console subsystem would flash a window.
    Assert-Equal (Get-PeSubsystem -Path $executablePath) 2 `
        'The executable must be a GUI-subsystem binary.'

    # No non-system UI framework is linked: the whole UI is raw Win32.
    $binaryText = [System.IO.File]::ReadAllText($executablePath, [System.Text.Encoding]::ASCII)
    foreach ($forbidden in @('WebView2', 'Microsoft.Web', 'Qt5', 'Qt6', 'gtk-3', 'wxmsw', 'Electron', 'CoreWebView')) {
        Assert-True (-not $binaryText.Contains($forbidden)) `
            "The executable must not link the $forbidden UI framework."
    }
    # No backend surface leaks into the shipped UI text.
    foreach ($forbidden in @('Save-and-Run', 'auto-login', 'SQLSTATE')) {
        Assert-True (-not $binaryText.Contains($forbidden)) `
            "The executable must not carry the string '$forbidden'."
    }

    # --- Fresh startup -----------------------------------------------------------------------
    $fresh = Invoke-AssembledUi -Root $packageRoot
    Assert-Equal $fresh.ExitCode 0 'A fresh start must exit cleanly on WM_CLOSE.'
    Assert-True ($fresh.Title.Length -gt 0) 'The UI must present a titled native window.'

    # The portable root is created exe-relative, not in AppData or the registry.
    foreach ($expected in @('data\.zeus-hso-root', 'data\state.sqlite3', 'data\vault.key')) {
        Assert-True (Test-Path -LiteralPath (Join-Path $packageRoot $expected)) `
            "Fresh startup must create $expected inside the package."
    }
    Assert-True (Test-Path -LiteralPath (Join-Path $packageRoot 'data\profiles') -PathType Container) `
        'Fresh startup must create the profiles directory.'

    # Nothing was written outside the package root.
    $strayRoots = @(Get-ChildItem -LiteralPath $workspace -Directory | Where-Object { $_.FullName -ne $packageRoot })
    Assert-Equal $strayRoots.Count 0 'Startup must not create directories outside the package root.'

    # --- Moved folder ------------------------------------------------------------------------
    Copy-Item -LiteralPath $packageRoot -Destination $movedRoot -Recurse
    $moved = Invoke-AssembledUi -Root $movedRoot
    Assert-Equal $moved.ExitCode 0 'A moved package must exit cleanly on WM_CLOSE.'
    Assert-Equal $moved.Title $fresh.Title 'A moved package must present the same native window.'

    # The moved copy keeps its own state; the original is untouched by the move.
    Assert-True (Test-Path -LiteralPath (Join-Path $movedRoot 'data\state.sqlite3')) `
        'The moved package must open its own database.'
    Assert-True (Test-Path -LiteralPath (Join-Path $packageRoot 'data\state.sqlite3')) `
        'The original package must remain intact after the copy.'

    Write-Output 'PASS: Windows account UI package contract'
}
finally {
    if (Test-Path -LiteralPath $workspace) {
        Remove-Item -LiteralPath $workspace -Recurse -Force -ErrorAction SilentlyContinue
    }
}
