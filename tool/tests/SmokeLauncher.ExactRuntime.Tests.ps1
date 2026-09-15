[CmdletBinding()]
param(
    [string] $RuntimeRoot = (Join-Path $PSScriptRoot '..\runtimes\windows-x64\temurin-11.0.32+9_microemu-2.0.4_ko402')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

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

$launcherPath = Join-Path $PSScriptRoot '..\launcher\Launch-Smoke.ps1'
if (-not (Test-Path -LiteralPath $launcherPath -PathType Leaf)) {
    throw "Exact-runtime launcher entry point is missing: $launcherPath"
}

$tokens = $null
$parseErrors = $null
$launcherAst = [Management.Automation.Language.Parser]::ParseFile(
    (Resolve-Path -LiteralPath $launcherPath).ProviderPath,
    [ref] $tokens,
    [ref] $parseErrors
)
if (@($parseErrors).Count -ne 0) {
    throw 'Exact-runtime launcher has PowerShell parse errors.'
}
$dryRunBranches = @($launcherAst.FindAll({
            param($node)
            $node -is [Management.Automation.Language.IfStatementAst] -and
            @($node.Clauses | Where-Object {
                    $_.Item1.Extent.Text -ceq '$DryRun'
                }).Count -eq 1
        }, $true))
Assert-Equal $dryRunBranches.Count 1 'Launcher must have one case-exact DryRun branch'
$dryRunExits = @($dryRunBranches[0].FindAll({
            param($node)
            $node -is [Management.Automation.Language.ExitStatementAst] -and
            $node.Extent.Text -ceq 'exit 0'
        }, $true))
Assert-Equal $dryRunExits.Count 1 'DryRun branch must exit successfully exactly once'
$javaConsoleInvocations = @($launcherAst.FindAll({
            param($node)
            $node -is [Management.Automation.Language.CommandAst] -and
            $node.InvocationOperator -eq
                [Management.Automation.Language.TokenKind]::Ampersand -and
            @($node.CommandElements).Count -gt 0 -and
            $node.CommandElements[0].Extent.Text -ceq '$spec.JavaConsoleExecutable'
        }, $true))
Assert-Equal $javaConsoleInvocations.Count 1 `
    'Launcher must retain one exact non-dry-run Java version invocation'
if ($dryRunBranches[0].Extent.EndOffset -ge
    $javaConsoleInvocations[0].Extent.StartOffset) {
    throw 'DryRun branch must exit before the Java console invocation.'
}

$profileId = '4c5f6da1-3b8a-4d61-bb6e-7bd8a64f0fa2'
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$dryRunDataRoot = Join-Path $tempBase ('zeus-exact-smoke-dry-run-' + [Guid]::NewGuid().ToString('N'))
try {
    $dryRunJson = & $launcherPath -RuntimeRoot $RuntimeRoot -DataRoot $dryRunDataRoot `
        -ProfileId $profileId -DryRun
    $dryRun = $dryRunJson | ConvertFrom-Json

    Assert-Equal $dryRun.RuntimeValid $true 'Dry run must validate the exact runtime tuple'
    Assert-Equal $dryRun.JavaVersion '11.0.32+9' 'Dry run must identify the pinned Java 11 runtime'
    Assert-Equal $dryRun.MicroemulatorSha256 `
        'dbd5f3eb8365d3e839d6a203149e0e3776fc1a0585e16ac1fc23f76c9fcae1c6' `
        'Dry run must verify the pinned MicroEmulator JAR'
    Assert-Equal $dryRun.GameSha256 `
        '6608bb0c77f03749e46165f711e9566dca4e172ce232256497b35faafe74c259' `
        'Dry run must verify the pinned game JAR'
    Assert-Equal (Test-Path -LiteralPath $dryRunDataRoot) $false `
        'Dry run must not create profile state'
}
finally {
    $resolvedDryRunRoot = [IO.Path]::GetFullPath($dryRunDataRoot)
    if (-not $resolvedDryRunRoot.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($resolvedDryRunRoot) -notlike 'zeus-exact-smoke-dry-run-*') {
        throw "Refusing to clean unexpected dry-run root: $resolvedDryRunRoot"
    }
    if (Test-Path -LiteralPath $resolvedDryRunRoot) {
        Remove-Item -LiteralPath $resolvedDryRunRoot -Recurse -Force
    }
}

Write-Output 'PASS: SmokeLauncher exact-runtime contract'
