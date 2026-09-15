#Requires -Version 7.0
<#
.SYNOPSIS
Captures one top-level window of this session to a PNG.

.DESCRIPTION
Development aid for verifying the shell by eye. It reads pixels only: no input is sent, no window is
moved or activated, and nothing outside the target window is captured.

PrintWindow with PW_RENDERFULLCONTENT is used rather than a screen grab so a window that is partly
covered, or behind another one, still yields its own current content.
#>
[CmdletBinding()]
param(
    # Substring of the window caption to capture. The first match wins.
    [Parameter(Mandatory, ParameterSetName = 'Caption')]
    [string] $TitleLike,

    # Window class to capture. Use this for a modal dialog, which is not any process's main window,
    # and for a caption whose diacritics a shell would mangle before this script sees them.
    [Parameter(Mandatory, ParameterSetName = 'Class')]
    [string] $ClassName,

    # Restricts a class capture to one process, when more than one package is running.
    [Parameter(ParameterSetName = 'Class')]
    [int] $OwnerProcessId,

    # PNG to write. An existing file is overwritten.
    [Parameter(Mandatory)]
    [string] $OutFile
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

Add-Type -AssemblyName System.Drawing

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class ZeusWindowCapture
{
    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr window, IntPtr deviceContext, uint flags);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr window, out Rect rect);

    [DllImport("user32.dll")]
    public static extern bool SetProcessDpiAwarenessContext(IntPtr context);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr FindWindowEx(
        IntPtr parent, IntPtr childAfter, string className, string windowName);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }

    // Renders the whole window, including content the compositor has not shown on screen.
    public const uint RenderFullContent = 2;

    // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2.
    public static readonly IntPtr PerMonitorAwareV2 = new IntPtr(-4);
}
'@

# Without this the host measures windows in virtualised coordinates on a scaled display, so a
# 1200-pixel window reports 960 and PrintWindow's real output is clipped to that smaller bitmap.
[void][ZeusWindowCapture]::SetProcessDpiAwarenessContext([ZeusWindowCapture]::PerMonitorAwareV2)

if ($PSCmdlet.ParameterSetName -eq 'Class') {
    $handle = [IntPtr]::Zero
    while ($true) {
        $handle = [ZeusWindowCapture]::FindWindowEx(
            [IntPtr]::Zero, $handle, $ClassName, [NullString]::Value)
        if ($handle -eq [IntPtr]::Zero) {
            throw "No window of class '$ClassName' was found."
        }
        if (-not $OwnerProcessId) {
            break
        }
        $owner = [uint32]0
        [void][ZeusWindowCapture]::GetWindowThreadProcessId($handle, [ref] $owner)
        if ([int]$owner -eq $OwnerProcessId) {
            break
        }
    }
}
else {
    $target = Get-Process |
        Where-Object { $_.MainWindowHandle -ne 0 -and $_.MainWindowTitle -like "*$TitleLike*" } |
        Select-Object -First 1
    if (-not $target) {
        throw "No window whose caption contains '$TitleLike' was found."
    }
    $handle = $target.MainWindowHandle
}

$rect = New-Object ZeusWindowCapture+Rect
# The window rect, not the client rect: PrintWindow renders the frame too, so a client-sized bitmap
# silently clips the right and bottom edges of the capture.
if (-not [ZeusWindowCapture]::GetWindowRect($handle, [ref] $rect)) {
    throw 'Could not measure the target window.'
}
$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
if ($width -le 0 -or $height -le 0) {
    throw "Target window has no drawable area: ${width}x${height}."
}

$bitmap = New-Object System.Drawing.Bitmap $width, $height
try {
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $deviceContext = $graphics.GetHdc()
        try {
            $captured = [ZeusWindowCapture]::PrintWindow(
                $handle,
                $deviceContext,
                [ZeusWindowCapture]::RenderFullContent)
        }
        finally {
            $graphics.ReleaseHdc($deviceContext)
        }
    }
    finally {
        $graphics.Dispose()
    }
    if (-not $captured) {
        throw 'PrintWindow refused to render the target window.'
    }
    $bitmap.Save($OutFile, [System.Drawing.Imaging.ImageFormat]::Png)
}
finally {
    $bitmap.Dispose()
}

[pscustomobject]@{
    Target  = if ($PSCmdlet.ParameterSetName -eq 'Class') { $ClassName } else { $target.MainWindowTitle }
    Width   = $width
    Height  = $height
    OutFile = (Resolve-Path -LiteralPath $OutFile).Path
}
