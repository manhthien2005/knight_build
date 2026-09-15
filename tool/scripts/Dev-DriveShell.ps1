#Requires -Version 7.0
<#
.SYNOPSIS
Drives the running Zeus shell by posted window messages, for by-eye verification.

.DESCRIPTION
Development aid only. It is not part of the product and nothing here is compiled into the tool: the
shell itself never synthesises input. This script exists so a panel or dialog can be brought on screen
for a capture without a human clicking through it.

It posts messages to the shell's own controls by their fixed child ids rather than moving the cursor or
using SendInput, so it cannot type into whatever window happens to be focused.

.PARAMETER Username
Account name to import. Use a synthetic value: this script is not a place for a real credential.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('AddAccount', 'SelectFirstRow', 'CheckFirstRow', 'RunChecked', 'StopChecked',
        'OpenConfig', 'ConfigRoundTrip')]
    [string] $Action,

    [string] $Username,

    [string] $Password,

    # Which shell instance to drive, when more than one is running. Without it the first window of
    # the class is used, which is arbitrary and can silently drive the wrong package.
    [int] $ProcessId
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class ZeusShellDriver
{
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr FindWindow(string className, string windowName);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr FindWindowEx(
        IntPtr parent, IntPtr childAfter, string className, string windowName);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    [DllImport("user32.dll")]
    public static extern IntPtr GetDlgItem(IntPtr window, int id);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wParam, string text);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "SendMessageW")]
    public static extern IntPtr SendMessageBuffer(
        IntPtr window, uint message, IntPtr wParam, StringBuilder text);

    [DllImport("user32.dll")]
    public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassName(IntPtr window, StringBuilder name, int capacity);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr window);

    [DllImport("user32.dll")]
    public static extern IntPtr SetFocus(IntPtr window);

    public const uint WM_SETTEXT = 0x000C;
    public const uint WM_GETTEXT = 0x000D;
    public const uint BM_GETCHECK = 0x00F0;
    public const uint CB_GETCURSEL = 0x0147;
    public const uint CB_SETCURSEL = 0x014E;
    public const uint WM_COMMAND = 0x0111;
    public const uint WM_KEYDOWN = 0x0100;
    public const uint WM_KEYUP = 0x0101;
    public const uint WM_LBUTTONDOWN = 0x0201;
    public const uint WM_LBUTTONUP = 0x0202;
    public const uint LVM_GETITEMCOUNT = 0x1004;
    public const uint LVM_GETITEMSTATE = 0x102C;
    public const int LVIS_STATEIMAGEMASK = 0xF000;
    public const int MK_LBUTTON = 0x0001;
    public const int VK_SPACE = 0x20;

    public const string ShellClass = "ZeusAccountManagerWindow";
    public const string DialogClass = "ZeusAccountManagerDialog";

    public const int ID_TABLE = 0x3001;
    public const int ID_USERNAME = 0x2001;
    public const int ID_PASSWORD = 0x2002;
    public const int ID_OK = 0x2005;
    public const int CMD_ADD = 0x1001;
    public const int CMD_RUN_SELECTED = 0x1002;
    public const int CMD_STOP_SELECTED = 0x1003;
    public const int CMD_ROW_CONFIG = 0x1106;
}
'@

function Find-ZeusWindow {
    param([Parameter(Mandatory)][string] $ClassName)
    # [NullString]::Value, not $null: PowerShell binds $null to a string parameter as the empty string,
    # and FindWindow then looks for a window whose caption is literally empty and finds nothing.
    if (-not $ProcessId) {
        return [ZeusShellDriver]::FindWindow($ClassName, [NullString]::Value)
    }
    # With two packages running, the first window of the class is arbitrary. Walking the class and
    # matching the owning process is what keeps a capture from driving the wrong instance.
    $window = [IntPtr]::Zero
    while ($true) {
        $window = [ZeusShellDriver]::FindWindowEx(
            [IntPtr]::Zero, $window, $ClassName, [NullString]::Value)
        if ($window -eq [IntPtr]::Zero) {
            return [IntPtr]::Zero
        }
        $owner = [uint32]0
        [void][ZeusShellDriver]::GetWindowThreadProcessId($window, [ref] $owner)
        if ([int]$owner -eq $ProcessId) {
            return $window
        }
    }
}

function Get-ShellWindow {
    $shell = Find-ZeusWindow -ClassName ([ZeusShellDriver]::ShellClass)
    if ($shell -eq [IntPtr]::Zero) {
        throw 'The Zeus shell window is not open.'
    }
    return $shell
}

function Invoke-FirstRowClick {
    param([Parameter(Mandatory)][int] $X)
    $shell = Get-ShellWindow
    $table = [ZeusShellDriver]::GetDlgItem($shell, [ZeusShellDriver]::ID_TABLE)
    if ($table -eq [IntPtr]::Zero) {
        throw 'The shell has no account table.'
    }
    $rows = [int][ZeusShellDriver]::SendMessage(
        $table, [ZeusShellDriver]::LVM_GETITEMCOUNT, [IntPtr]::Zero, [IntPtr]::Zero)
    if ($rows -lt 1) {
        throw 'The account table is empty, so no row can be clicked.'
    }
    # Row 0 sits just under the header; 12 pixels down the client area lands inside it.
    $point = [IntPtr]((12 -shl 16) -bor $X)
    [void][ZeusShellDriver]::PostMessage(
        $table, [ZeusShellDriver]::WM_LBUTTONDOWN, [IntPtr][ZeusShellDriver]::MK_LBUTTON, $point)
    [void][ZeusShellDriver]::PostMessage(
        $table, [ZeusShellDriver]::WM_LBUTTONUP, [IntPtr]::Zero, $point)
    Start-Sleep -Milliseconds 400
    return [pscustomobject]@{ Action = $Action; Rows = $rows }
}

function Open-ConfigDialog {
    # The settings dialog follows the highlighted row, so the highlight is moved first.
    $null = Invoke-FirstRowClick -X 140
    $shell = Get-ShellWindow
    [void][ZeusShellDriver]::PostMessage(
        $shell,
        [ZeusShellDriver]::WM_COMMAND,
        [IntPtr][ZeusShellDriver]::CMD_ROW_CONFIG,
        [IntPtr]::Zero)
    foreach ($attempt in 1..40) {
        Start-Sleep -Milliseconds 150
        $dialog = Find-ZeusWindow -ClassName ([ZeusShellDriver]::DialogClass)
        if ($dialog -ne [IntPtr]::Zero) {
            return $dialog
        }
    }
    throw 'The settings dialog did not open.'
}

function Get-ControlText {
    param(
        [Parameter(Mandatory)][IntPtr] $Dialog,
        [Parameter(Mandatory)][int] $Id)
    $control = [ZeusShellDriver]::GetDlgItem($Dialog, $Id)
    if ($control -eq [IntPtr]::Zero) {
        return $null
    }
    $buffer = New-Object System.Text.StringBuilder 64
    [void][ZeusShellDriver]::SendMessageBuffer(
        $control, [ZeusShellDriver]::WM_GETTEXT, [IntPtr]64, $buffer)
    return $buffer.ToString()
}

switch ($Action) {
    'AddAccount' {
        if (-not $Username -or -not $Password) {
            throw 'AddAccount needs both Username and Password.'
        }
        $shell = Get-ShellWindow
        [void][ZeusShellDriver]::PostMessage(
            $shell, [ZeusShellDriver]::WM_COMMAND, [IntPtr][ZeusShellDriver]::CMD_ADD, [IntPtr]::Zero)

        $dialog = [IntPtr]::Zero
        foreach ($attempt in 1..40) {
            Start-Sleep -Milliseconds 150
            $dialog = Find-ZeusWindow -ClassName ([ZeusShellDriver]::DialogClass)
            if ($dialog -ne [IntPtr]::Zero) { break }
        }
        if ($dialog -eq [IntPtr]::Zero) {
            throw 'The add-account dialog did not open.'
        }

        foreach ($field in @(
            @{ Id = [ZeusShellDriver]::ID_USERNAME; Value = $Username },
            @{ Id = [ZeusShellDriver]::ID_PASSWORD; Value = $Password })) {
            $control = [ZeusShellDriver]::GetDlgItem($dialog, $field.Id)
            if ($control -eq [IntPtr]::Zero) {
                throw "The dialog has no field $($field.Id)."
            }
            [void][ZeusShellDriver]::SendMessage(
                $control, [ZeusShellDriver]::WM_SETTEXT, [IntPtr]::Zero, $field.Value)
        }
        [void][ZeusShellDriver]::PostMessage(
            $dialog, [ZeusShellDriver]::WM_COMMAND, [IntPtr][ZeusShellDriver]::ID_OK, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 800

        $table = [ZeusShellDriver]::GetDlgItem($shell, [ZeusShellDriver]::ID_TABLE)
        $rows = [ZeusShellDriver]::SendMessage(
            $table, [ZeusShellDriver]::LVM_GETITEMCOUNT, [IntPtr]::Zero, [IntPtr]::Zero)
        [pscustomobject]@{ Action = $Action; Rows = [int]$rows }
    }
    'SelectFirstRow' {
        # A click well right of the checkbox column, on the first row: clicking the checkbox would
        # toggle batch selection instead of moving the highlight the panel follows.
        Invoke-FirstRowClick -X 140
    }
    'CheckFirstRow' {
        # Space toggles the focused item's checkbox. Preferred over clicking the state image: the
        # checkbox hit rectangle moves with DPI, and a pixel that misses it silently does nothing.
        $shell = Get-ShellWindow
        $table = [ZeusShellDriver]::GetDlgItem($shell, [ZeusShellDriver]::ID_TABLE)
        if ($table -eq [IntPtr]::Zero) {
            throw 'The shell has no account table.'
        }
        $null = Invoke-FirstRowClick -X 140
        [void][ZeusShellDriver]::SetForegroundWindow($shell)
        [void][ZeusShellDriver]::SetFocus($table)
        Start-Sleep -Milliseconds 200
        [void][ZeusShellDriver]::PostMessage(
            $table, [ZeusShellDriver]::WM_KEYDOWN, [IntPtr][ZeusShellDriver]::VK_SPACE, [IntPtr]::Zero)
        [void][ZeusShellDriver]::PostMessage(
            $table, [ZeusShellDriver]::WM_KEYUP, [IntPtr][ZeusShellDriver]::VK_SPACE, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 400
        $state = [int][ZeusShellDriver]::SendMessage(
            $table,
            [ZeusShellDriver]::LVM_GETITEMSTATE,
            [IntPtr]0,
            [IntPtr][ZeusShellDriver]::LVIS_STATEIMAGEMASK)
        [pscustomobject]@{ Action = $Action; Checked = (($state -shr 12) -eq 2) }
    }
    'RunChecked' {
        $shell = Get-ShellWindow
        [void][ZeusShellDriver]::PostMessage(
            $shell,
            [ZeusShellDriver]::WM_COMMAND,
            [IntPtr][ZeusShellDriver]::CMD_RUN_SELECTED,
            [IntPtr]::Zero)
        Start-Sleep -Milliseconds 1500
        [pscustomobject]@{ Action = $Action }
    }
    'OpenConfig' {
        $dialog = Open-ConfigDialog
        # Reported so a missing control shows up here rather than as a silently unread setting.
        $present = @()
        foreach ($id in @(0x2020, 0x2021, 0x2022, 0x2023, 0x2024, 0x2025, 0x2026, 0x2027, 0x2028,
                0x2029, 0x202a, 0x202b, 0x2030, 0x2031, 0x2032, 0x2034, 0x2035, 0x2039)) {
            if ([ZeusShellDriver]::GetDlgItem($dialog, $id) -ne [IntPtr]::Zero) {
                $present += ('0x{0:x}' -f $id)
            }
        }
        [pscustomobject]@{ Action = $Action; Controls = $present.Count; Present = $present }
    }
    'ConfigRoundTrip' {
        # Proves the settings surface end to end without a game: type values, save, reopen, read back.
        # The mode is deliberately left off — arming needs a live character, so a mode change here
        # would be refused and nothing would persist to compare.
        $edits = @{ 0x2024 = '200'; 0x2027 = '70'; 0x2028 = '25'; 0x2037 = '3' }
        $choices = @{ 0x2029 = 3; 0x202a = 1; 0x202b = 1; 0x2036 = 1 }
        $checks = @(0x2030, 0x2035, 0x2038, 0x2039, 0x2040, 0x2042)

        $dialog = Open-ConfigDialog
        foreach ($id in $edits.Keys) {
            $control = [ZeusShellDriver]::GetDlgItem($dialog, $id)
            if ($control -eq [IntPtr]::Zero) { throw "No settings field 0x$('{0:x}' -f $id)." }
            [void][ZeusShellDriver]::SendMessage(
                $control, [ZeusShellDriver]::WM_SETTEXT, [IntPtr]::Zero, $edits[$id])
        }
        foreach ($id in $choices.Keys) {
            $control = [ZeusShellDriver]::GetDlgItem($dialog, $id)
            if ($control -eq [IntPtr]::Zero) { throw "No settings picker 0x$('{0:x}' -f $id)." }
            [void][ZeusShellDriver]::SendMessage(
                $control, [ZeusShellDriver]::CB_SETCURSEL, [IntPtr]$choices[$id], [IntPtr]::Zero)
        }
        foreach ($id in $checks) {
            # Posted as a click, so the checkbox flips itself the way the operator's click would.
            $control = [ZeusShellDriver]::GetDlgItem($dialog, $id)
            if ($control -eq [IntPtr]::Zero) { throw "No settings check 0x$('{0:x}' -f $id)." }
            [void][ZeusShellDriver]::PostMessage(
                $control, [ZeusShellDriver]::WM_LBUTTONDOWN,
                [IntPtr][ZeusShellDriver]::MK_LBUTTON, [IntPtr]0)
            [void][ZeusShellDriver]::PostMessage(
                $control, [ZeusShellDriver]::WM_LBUTTONUP, [IntPtr]::Zero, [IntPtr]0)
        }
        Start-Sleep -Milliseconds 400
        [void][ZeusShellDriver]::PostMessage(
            $dialog, [ZeusShellDriver]::WM_COMMAND, [IntPtr][ZeusShellDriver]::ID_OK, [IntPtr]::Zero)
        # The write goes to the worker, so the reopen has to wait for its reply to land.
        Start-Sleep -Milliseconds 1500

        $dialog = Open-ConfigDialog
        $readback = [ordered]@{}
        foreach ($id in $edits.Keys) {
            $readback['0x{0:x}' -f $id] = Get-ControlText -Dialog $dialog -Id $id
        }
        foreach ($id in $choices.Keys) {
            $control = [ZeusShellDriver]::GetDlgItem($dialog, $id)
            $readback['0x{0:x}' -f $id] = [int][ZeusShellDriver]::SendMessage(
                $control, [ZeusShellDriver]::CB_GETCURSEL, [IntPtr]::Zero, [IntPtr]::Zero)
        }
        foreach ($id in $checks) {
            $control = [ZeusShellDriver]::GetDlgItem($dialog, $id)
            $readback['0x{0:x}' -f $id] = [int][ZeusShellDriver]::SendMessage(
                $control, [ZeusShellDriver]::BM_GETCHECK, [IntPtr]::Zero, [IntPtr]::Zero)
        }
        [pscustomobject]$readback
    }
    'StopChecked' {
        $shell = Get-ShellWindow
        [void][ZeusShellDriver]::PostMessage(
            $shell,
            [ZeusShellDriver]::WM_COMMAND,
            [IntPtr][ZeusShellDriver]::CMD_STOP_SELECTED,
            [IntPtr]::Zero)
        Start-Sleep -Milliseconds 2000
        [pscustomobject]@{ Action = $Action }
    }
}
