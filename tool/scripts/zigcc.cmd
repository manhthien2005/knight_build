@echo off
setlocal
"%~dp0..\.devtools\zig-0.16.0\zig.exe" cc -target x86_64-windows-gnu %* -fno-sanitize=undefined
