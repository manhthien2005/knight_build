$ErrorActionPreference = 'Stop'
$CargoArguments = $args
$projectRoot = Split-Path -Parent $PSScriptRoot
$localCargo = Join-Path $projectRoot '.devtools\cargo'
if (Test-Path -LiteralPath $localCargo -PathType Container) {
    $env:RUSTUP_HOME = Join-Path $projectRoot '.devtools\rustup'
    $env:CARGO_HOME = $localCargo
    $cargo = Join-Path $env:CARGO_HOME 'bin\cargo.exe'
    $rustc = Join-Path $env:CARGO_HOME 'bin\rustc.exe'
} else {
    $cargo = (Get-Command cargo.exe -ErrorAction SilentlyContinue).Source
    $rustc = (Get-Command rustc.exe -ErrorAction SilentlyContinue).Source
}
$env:ZIG_GLOBAL_CACHE_DIR = Join-Path $projectRoot '.devtools\zig-global-cache'
$env:ZIG_LOCAL_CACHE_DIR = Join-Path $projectRoot 'target\zig-cache'
$env:CC_x86_64_pc_windows_gnu = Join-Path $PSScriptRoot 'zigcc.cmd'
$env:AR_x86_64_pc_windows_gnu = Join-Path $PSScriptRoot 'zigar.cmd'
$env:CRATE_CC_NO_DEFAULTS = '1'
$env:RUSTFLAGS = '-C link-self-contained=yes'
$shimSource = Join-Path $PSScriptRoot 'tool-shims\dlltool.rs'
$shimDirectory = Join-Path $projectRoot '.devtools\bin'
$dlltoolShim = Join-Path $shimDirectory 'dlltool.exe'

if (-not $cargo -or -not (Test-Path -LiteralPath $cargo -PathType Leaf)) {
    throw "Cargo toolchain is missing. See README.md for bootstrap details."
}

New-Item -ItemType Directory -Path $shimDirectory -Force | Out-Null
$shimNeedsBuild = -not (Test-Path -LiteralPath $dlltoolShim -PathType Leaf)
if (-not $shimNeedsBuild) {
    $shimNeedsBuild = (Get-Item -LiteralPath $shimSource).LastWriteTimeUtc -gt
        (Get-Item -LiteralPath $dlltoolShim).LastWriteTimeUtc
}
if ($shimNeedsBuild) {
    & $rustc $shimSource '--edition=2024' '-C' 'opt-level=s' '-o' $dlltoolShim
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

$env:PATH = "$shimDirectory;$env:PATH"

& $cargo @CargoArguments
exit $LASTEXITCODE
