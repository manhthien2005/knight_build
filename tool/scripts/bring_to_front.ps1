Add-Type @'
using System;
using System.Runtime.InteropServices;
public class AllWinFinder {
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
    public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder lpString, int nMaxCount);
    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
    public static extern int GetClassName(IntPtr hWnd, System.Text.StringBuilder lpClassName, int nMaxCount);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);
}
'@

[AllWinFinder]::EnumWindows({
    param($hwnd, $lparam)
    $sbText = New-Object System.Text.StringBuilder 256
    [void][AllWinFinder]::GetWindowText($hwnd, $sbText, 256)
    $title = $sbText.ToString()
    $sbClass = New-Object System.Text.StringBuilder 256
    [void][AllWinFinder]::GetClassName($hwnd, $sbClass, 256)
    $class = $sbClass.ToString()
    if ($title -like "*Zeus*" -or $class -like "*Zeus*") {
        $p = [uint32]0
        [void][AllWinFinder]::GetWindowThreadProcessId($hwnd, [ref]$p)
        Write-Host "FOUND: HWND: $hwnd | PID: $p | Class: '$class' | Title: '$title'"
        [AllWinFinder]::ShowWindow($hwnd, 9)
        [AllWinFinder]::SetForegroundWindow($hwnd)
    }
    return $true
}, [IntPtr]::Zero)
