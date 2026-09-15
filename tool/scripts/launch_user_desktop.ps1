$ErrorActionPreference = 'Stop'

# Kill any running instances
Stop-Process -Name zeus-ui -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 200

# Remove instance lock file
Remove-Item 'D:\Gaming\KnightOnline_402\ZeusPlay\data\.core-instance.lock' -Force -ErrorAction SilentlyContinue

$csSource = @"
using System;
using System.Runtime.InteropServices;

public class UserDesktopRunner {
    [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Auto)]
    public static extern bool CreateProcess(
        string lpApplicationName,
        string lpCommandLine,
        IntPtr lpProcessAttributes,
        IntPtr lpThreadAttributes,
        bool bInheritHandles,
        uint dwCreationFlags,
        IntPtr lpEnvironment,
        string lpCurrentDirectory,
        ref STARTUPINFO lpStartupInfo,
        out PROCESS_INFORMATION lpProcessInformation
    );

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Auto)]
    public struct STARTUPINFO {
        public int cb;
        public string lpReserved;
        public string lpDesktop;
        public string lpTitle;
        public int dwX;
        public int dwY;
        public int dwXSize;
        public int dwYSize;
        public int dwXCountChars;
        public int dwYCountChars;
        public int dwFillAttribute;
        public int dwFlags;
        public short wShowWindow;
        public short cbReserved2;
        public IntPtr lpReserved2;
        public IntPtr hStdInput;
        public IntPtr hStdOutput;
        public IntPtr hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct PROCESS_INFORMATION {
        public IntPtr hProcess;
        public IntPtr hThread;
        public int dwProcessId;
        public int dwThreadId;
    }

    public static int Launch(string appPath, string workDir) {
        STARTUPINFO si = new STARTUPINFO();
        si.cb = Marshal.SizeOf(si);
        si.lpDesktop = @"WinSta0\Default";
        PROCESS_INFORMATION pi = new PROCESS_INFORMATION();
        bool success = CreateProcess(null, "\"" + appPath + "\"", IntPtr.Zero, IntPtr.Zero, false, 0, IntPtr.Zero, workDir, ref si, out pi);
        return success ? pi.dwProcessId : -1;
    }
}
"@

Add-Type -TypeDefinition $csSource
$launchedPid = [UserDesktopRunner]::Launch('D:\Gaming\KnightOnline_402\ZeusPlay\zeus-ui.exe', 'D:\Gaming\KnightOnline_402\ZeusPlay')
Write-Host "Zeus UI launched on interactive user desktop (WinSta0\Default) with PID: $launchedPid"
