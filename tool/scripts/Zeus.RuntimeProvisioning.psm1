Set-StrictMode -Version Latest

$script:MaximumDescriptorBytes = 64KB
$script:MaximumManifestBytes = 2MB
$script:MaximumJreFiles = 10000
$script:MaximumJreBytes = 512MB
$script:MaximumJreTreeEntries = 20000
$script:MaximumJreTreeDepth = 64
$script:MaximumRuntimeTreeEntries = 22000
$script:MaximumZipEntries = 10000
$script:MaximumZipNodes = 20000
$script:MaximumZipDepth = 64
$script:MaximumZipCentralDirectoryBytes = 16MB
$script:MaximumStagingTreeEntries = 45000
$script:MaximumStagingTreeDepth = 68
$script:MaximumJreArchiveExpandedBytes = 600MB
$script:MaximumMicroemulatorArchiveExpandedBytes = 64MB
$script:MaximumArchiveBytes = 512MB
$script:StreamBufferBytes = 64KB

if (-not ('ZeusRuntimeNativeSecurity' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using Microsoft.Win32.SafeHandles;
using System.Runtime.InteropServices;
using System.Text;

public static class ZeusRuntimeNativeSecurity
{
    [StructLayout(LayoutKind.Sequential)]
    public struct SecurityAttributes
    {
        public int Length;
        public IntPtr SecurityDescriptor;
        public int InheritHandle;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct FileInformation
    {
        public uint FileAttributes;
        public System.Runtime.InteropServices.ComTypes.FILETIME CreationTime;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastAccessTime;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWriteTime;
        public uint VolumeSerialNumber;
        public uint FileSizeHigh;
        public uint FileSizeLow;
        public uint NumberOfLinks;
        public uint FileIndexHigh;
        public uint FileIndexLow;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool CreateDirectoryW(
        string path,
        ref SecurityAttributes securityAttributes);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern SafeFileHandle CreateFileW(
        string path,
        uint desiredAccess,
        uint shareMode,
        IntPtr securityAttributes,
        uint creationDisposition,
        uint flagsAndAttributes,
        IntPtr templateFile);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetFileInformationByHandle(
        SafeFileHandle handle,
        out FileInformation information);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern uint GetFinalPathNameByHandleW(
        SafeFileHandle handle,
        StringBuilder path,
        uint pathLength,
        uint flags);
}
'@
}

function Invoke-ZeusRuntimeProvisioning {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string] $RuntimeRoot,

        [Parameter(Mandatory)]
        [string] $CacheRoot,

        [string] $GameJar,
        [string] $JreArchive,
        [string] $MicroemulatorArchive,
        [switch] $VerifyOnly
    )

    $runtimeFull = Normalize-ZeusDirectoryPath -Path $RuntimeRoot
    Assert-ZeusLocalPlainPath -Path $runtimeFull -Label 'runtime root'
    Assert-ZeusPlainDirectory -Path $runtimeFull -Label 'runtime root'
    $runtimePin = Open-ZeusPinnedDirectoryHandle -Path $runtimeFull -Label 'runtime root'
    $bootstrap = $null
    try {
        if ($VerifyOnly) {
            Assert-ZeusAclTree -Path $runtimeFull -Label 'runtime security'
            $context = Read-ZeusRuntimeContext -RuntimeRoot $runtimeFull
        }
        else {
            $bootstrap = Open-ZeusRuntimeBootstrap -RuntimeRoot $runtimeFull
            try {
                Protect-ZeusAclTree -Path $runtimeFull -Label 'runtime security'
                Assert-ZeusPinnedDirectoryIdentity -Pin $runtimePin -Label 'runtime root'
                $context = Read-ZeusRuntimeContext -RuntimeRoot $runtimeFull
                Assert-ZeusRuntimeBootstrapUnchanged -Bootstrap $bootstrap -Context $context
            }
            finally {
                Close-ZeusRuntimeBootstrap -Bootstrap $bootstrap
                $bootstrap = $null
            }
        }

        try {
            $verified = Test-ZeusRuntimePayload -Context $context
            $status = if ($VerifyOnly) { 'verified' } else { 'already_provisioned' }
            return New-ZeusProvisioningResult -Status $status -Verified $verified
        }
        catch {
            if ($VerifyOnly) {
                throw
            }
            $initialFailure = $_.Exception.Message
        }

        $payloadPaths = @(
            (Join-Path $context.root 'jre'),
            (Resolve-ZeusRelativePath -Root $context.root -RelativePath ([string] $context.descriptor.microemulator.jar) -Label 'MicroEmulator JAR' | Split-Path -Parent),
            (Resolve-ZeusRelativePath -Root $context.root -RelativePath ([string] $context.descriptor.game.jar) -Label 'Game JAR' | Split-Path -Parent)
        )
        $existingPayloadPaths = @($payloadPaths | Where-Object { Test-Path -LiteralPath $_ })
        if ($existingPayloadPaths.Count -gt 0) {
            throw "Existing runtime payload failed verification; refusing to overwrite partial or corrupt content: $initialFailure"
        }

        return Invoke-ZeusFreshRuntimeProvisioning -Context $context -CacheRoot $CacheRoot `
            -RuntimePin $runtimePin -GameJar $GameJar -JreArchive $JreArchive `
            -MicroemulatorArchive $MicroemulatorArchive
    }
    finally {
        if ($null -ne $bootstrap) {
            Close-ZeusRuntimeBootstrap -Bootstrap $bootstrap
        }
        try {
            Assert-ZeusPinnedDirectoryIdentity -Pin $runtimePin -Label 'runtime root'
        }
        finally {
            $runtimePin.handle.Dispose()
        }
    }
}

function Invoke-ZeusFreshRuntimeProvisioning {
    param(
        [Parameter(Mandatory)] $Context,
        [Parameter(Mandatory)] [string] $CacheRoot,
        [Parameter(Mandatory)] $RuntimePin,
        [string] $GameJar,
        [string] $JreArchive,
        [string] $MicroemulatorArchive
    )

    if ([string]::IsNullOrWhiteSpace($GameJar)) {
        throw 'A fresh runtime requires -GameJar with an operator-supplied legal game JAR.'
    }
    $cacheBoundary = Open-ZeusRuntimeCacheBoundary -RuntimePin $RuntimePin `
        -CacheRoot $CacheRoot
    try {
        $gameSource = [IO.Path]::GetFullPath($GameJar)
        Assert-ZeusLocalPlainPath -Path $gameSource -Label 'Game JAR'
        $gameHandle = Open-ZeusPinnedReadHandle -Path $gameSource `
            -ExpectedSize ([int64] $Context.descriptor.game.jar_size) `
            -ExpectedSha256 ([string] $Context.descriptor.game.jar_sha256) -Label 'Game JAR'
        try {
            $cacheInitialization = Initialize-ZeusPinnedPrivateCacheRoot `
                -Boundary $cacheBoundary
            $cacheFull = $cacheInitialization.path
            $cachePin = $cacheInitialization.cache_pin
            try {
                Assert-ZeusPinnedPrivateDirectory -Pin $cachePin -Label 'runtime cache security'
                Assert-ZeusPinnedDirectoryIdentity -Pin $cacheBoundary.ancestor_pin `
                    -Label 'runtime cache existing ancestor'
                if (-not $cachePin.final_path.Equals(
                    $cacheBoundary.canonical_cache_path,
                    [StringComparison]::OrdinalIgnoreCase
                )) {
                    throw 'Runtime cache canonical path changed while its boundary was established.'
                }
                $stagingRoot = Join-Path $cacheFull ('.staging-' + [Guid]::NewGuid().ToString('N'))
                $stagingPin = $null
                $jreArchiveHandle = $null
                $microemulatorArchiveHandle = $null
                try {
                    $jreArchiveHandle = Get-ZeusPinnedArchiveHandle -OverridePath $JreArchive `
                        -CacheRoot $cacheFull `
                        -ArchiveName ([string] $Context.descriptor.java.archive_name) `
                        -ExpectedSize ([int64] $Context.descriptor.java.archive_size) `
                        -ExpectedSha256 ([string] $Context.descriptor.java.archive_sha256) `
                        -Source ([string] $Context.descriptor.java.source) -Label 'JRE archive'
                    $microemulatorArchiveHandle = Get-ZeusPinnedArchiveHandle `
                        -OverridePath $MicroemulatorArchive -CacheRoot $cacheFull `
                        -ArchiveName ([string] $Context.descriptor.microemulator.archive_name) `
                        -ExpectedSize ([int64] $Context.descriptor.microemulator.archive_size) `
                        -ExpectedSha256 ([string] $Context.descriptor.microemulator.archive_sha256) `
                        -Source ([string] $Context.descriptor.microemulator.source) `
                        -Label 'MicroEmulator archive'

                    New-ZeusPrivateDirectory -Path $stagingRoot -Label 'runtime staging root'
                    $stagingPin = Open-ZeusPinnedDirectoryHandle -Path $stagingRoot `
                        -Label 'runtime staging root'
                    Assert-ZeusPinnedPrivateDirectory -Pin $stagingPin `
                        -Label 'runtime staging security'
                    $jreExtracted = Join-Path $stagingRoot 'jre-extracted'
                    $microemulatorExtracted = Join-Path $stagingRoot 'microemulator-extracted'
                    Expand-ZeusSafeZip -ArchiveStream $jreArchiveHandle.stream `
                        -Destination $jreExtracted `
                        -MaximumExpandedBytes $script:MaximumJreArchiveExpandedBytes `
                        -Label 'JRE archive'
                    Expand-ZeusSafeZip -ArchiveStream $microemulatorArchiveHandle.stream `
                        -Destination $microemulatorExtracted `
                        -MaximumExpandedBytes $script:MaximumMicroemulatorArchiveExpandedBytes `
                        -Label 'MicroEmulator archive'

                    $payloadRoot = Join-Path $stagingRoot 'payload'
                    New-ZeusPrivateDirectory -Path $payloadRoot -Label 'staged payload root'
                    Copy-Item -LiteralPath $Context.descriptor_path `
                        -Destination (Join-Path $payloadRoot 'runtime-descriptor.json')
                    Copy-Item -LiteralPath $Context.manifest_path `
                        -Destination (Join-Path $payloadRoot ([string] $Context.descriptor.java.tree_manifest))

                    $jreTopLevel = @(Get-ChildItem -LiteralPath $jreExtracted -Force)
                    if ($jreTopLevel.Count -ne 1 -or -not $jreTopLevel[0].PSIsContainer -or
                        (($jreTopLevel[0].Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
                        throw 'JRE archive must contain exactly one plain top-level directory.'
                    }
                    Move-Item -LiteralPath $jreTopLevel[0].FullName `
                        -Destination (Join-Path $payloadRoot 'jre')

                    $microemulatorCandidates = @(
                        Get-ChildItem -LiteralPath $microemulatorExtracted -File -Recurse -Force |
                            Where-Object {
                                $_.Name -eq [IO.Path]::GetFileName(
                                    [string] $Context.descriptor.microemulator.jar
                                )
                            }
                    )
                    if ($microemulatorCandidates.Count -ne 1) {
                        throw 'MicroEmulator archive must contain exactly one pinned JAR candidate.'
                    }
                    $stagedMicroemulator = Resolve-ZeusRelativePath -Root $payloadRoot `
                        -RelativePath ([string] $Context.descriptor.microemulator.jar) `
                        -Label 'MicroEmulator JAR'
                    New-Item -ItemType Directory -Path (Split-Path -Parent $stagedMicroemulator) |
                        Out-Null
                    Copy-Item -LiteralPath $microemulatorCandidates[0].FullName `
                        -Destination $stagedMicroemulator

                    $stagedGame = Resolve-ZeusRelativePath -Root $payloadRoot `
                        -RelativePath ([string] $Context.descriptor.game.jar) -Label 'Game JAR'
                    New-Item -ItemType Directory -Path (Split-Path -Parent $stagedGame) | Out-Null
                    Copy-ZeusHeldFileToNewPath -Handle $gameHandle -Destination $stagedGame `
                        -ExpectedSize ([int64] $Context.descriptor.game.jar_size) -Label 'Game JAR'

                    Protect-ZeusAclTree -Path $payloadRoot -Label 'staged runtime security'
                    $stagedContext = Read-ZeusRuntimeContext -RuntimeRoot $payloadRoot
                    Test-ZeusRuntimePayload -Context $stagedContext | Out-Null

                    Install-ZeusStagedPayload -StagedRoot $payloadRoot `
                        -RuntimeRoot $Context.root
                    Protect-ZeusAclTree -Path $Context.root -Label 'installed runtime security'

                    $installedContext = Read-ZeusRuntimeContext -RuntimeRoot $Context.root
                    $installed = Test-ZeusRuntimePayload -Context $installedContext
                    return New-ZeusProvisioningResult -Status 'provisioned' -Verified $installed
                }
                finally {
                    if ($null -ne $microemulatorArchiveHandle) {
                        $microemulatorArchiveHandle.stream.Dispose()
                    }
                    if ($null -ne $jreArchiveHandle) {
                        $jreArchiveHandle.stream.Dispose()
                    }
                    if ($null -ne $stagingPin) {
                        try {
                            Assert-ZeusPinnedDirectoryIdentity -Pin $stagingPin `
                                -Label 'runtime staging root'
                        }
                        finally {
                            $stagingPin.handle.Dispose()
                        }
                    }
                    if (Test-Path -LiteralPath $stagingRoot) {
                        Remove-ZeusStagingDirectory -StagingRoot $stagingRoot -CacheRoot $cacheFull
                    }
                }
            }
            finally {
                try {
                    Assert-ZeusPinnedDirectoryIdentity -Pin $cachePin -Label 'runtime cache'
                }
                finally {
                    $cachePin.handle.Dispose()
                    Close-ZeusPinnedDirectoryChain -Pins $cacheInitialization.parent_pins `
                        -Label 'runtime cache parent'
                }
            }
        }
        finally {
            $gameHandle.stream.Dispose()
        }
    }
    finally {
        try {
            Assert-ZeusPinnedDirectoryIdentity -Pin $cacheBoundary.ancestor_pin `
                -Label 'runtime cache existing ancestor'
        }
        finally {
            $cacheBoundary.ancestor_pin.handle.Dispose()
        }
    }
}

function Read-ZeusHeldStreamBytes {
    param(
        [Parameter(Mandatory)] [IO.FileStream] $Stream,
        [Parameter(Mandatory)] [int64] $MaximumBytes,
        [Parameter(Mandatory)] [string] $Label
    )

    if ($Stream.Length -gt $MaximumBytes) {
        throw "$Label exceeds its bounded size limit."
    }
    $Stream.Position = 0
    $output = [IO.MemoryStream]::new()
    try {
        Copy-ZeusBoundedStream -InputStream $Stream -OutputStream $output `
            -MaximumBytes $MaximumBytes -ExpectedBytes $Stream.Length -Label $Label | Out-Null
        $bytes = $output.ToArray()
    }
    finally {
        $output.Dispose()
        $Stream.Position = 0
    }
    Write-Output -NoEnumerate $bytes
}

function Get-ZeusRequiredDescriptorProperty {
    param(
        $InputObject,
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [string] $Label
    )

    if ($null -eq $InputObject) {
        throw "$Label is missing."
    }
    $property = $InputObject.PSObject.Properties[$Name]
    if ($null -eq $property -or $null -eq $property.Value) {
        throw "$Label is missing."
    }
    $property.Value
}

function Assert-ZeusDescriptorText {
    param(
        $Value,
        [Parameter(Mandatory)] [string] $Label
    )

    $text = [string] $Value
    if ([string]::IsNullOrWhiteSpace($text)) {
        throw "$Label is missing."
    }
    $text
}

function Assert-ZeusDescriptorInteger {
    param(
        $Value,
        [Parameter(Mandatory)] [int64] $Minimum,
        [Parameter(Mandatory)] [int64] $Maximum,
        [Parameter(Mandatory)] [string] $Label
    )

    $parsed = [int64] 0
    $text = [Convert]::ToString($Value, [Globalization.CultureInfo]::InvariantCulture)
    if (-not [int64]::TryParse(
        $text,
        [Globalization.NumberStyles]::None,
        [Globalization.CultureInfo]::InvariantCulture,
        [ref] $parsed
    ) -or $parsed -lt $Minimum -or $parsed -gt $Maximum) {
        throw "$Label is invalid."
    }
    $parsed
}

function Assert-ZeusDescriptorSha256 {
    param(
        $Value,
        [Parameter(Mandatory)] [string] $Label
    )

    $sha256 = [string] $Value
    if ($sha256 -notmatch '^[0-9a-f]{64}$') {
        throw "$Label descriptor SHA-256 is invalid."
    }
    $sha256
}

function Assert-ZeusRuntimeDescriptor {
    param(
        [Parameter(Mandatory)] $Descriptor,
        [Parameter(Mandatory)] [string] $Root
    )

    $schemaVersion = Get-ZeusRequiredDescriptorProperty -InputObject $Descriptor `
        -Name 'schema_version' -Label 'Runtime descriptor schema_version'
    if ((Assert-ZeusDescriptorInteger -Value $schemaVersion -Minimum 1 -Maximum 1 `
        -Label 'Runtime descriptor schema_version') -ne 1) {
        throw 'Runtime descriptor schema_version must be 1.'
    }
    $runtimeId = Assert-ZeusDescriptorText -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $Descriptor -Name 'runtime_id' `
            -Label 'Runtime descriptor runtime_id'
    ) -Label 'Runtime descriptor runtime_id'
    if ($runtimeId.Length -gt 256) {
        throw 'Runtime descriptor runtime_id is too long.'
    }

    $platform = Get-ZeusRequiredDescriptorProperty -InputObject $Descriptor `
        -Name 'platform' -Label 'Runtime descriptor platform'
    $operatingSystem = Assert-ZeusDescriptorText -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $platform -Name 'os' `
            -Label 'Runtime descriptor platform.os'
    ) -Label 'Runtime descriptor platform.os'
    $architecture = Assert-ZeusDescriptorText -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $platform -Name 'architecture' `
            -Label 'Runtime descriptor platform.architecture'
    ) -Label 'Runtime descriptor platform.architecture'
    if ($operatingSystem -ne 'windows' -or $architecture -ne 'x64') {
        throw 'This provisioner only accepts the Windows x64 exact-runtime tuple.'
    }

    $java = Get-ZeusRequiredDescriptorProperty -InputObject $Descriptor `
        -Name 'java' -Label 'Runtime descriptor java'
    $jreArchiveName = Assert-ZeusArchiveName -ArchiveName (
        Get-ZeusRequiredDescriptorProperty -InputObject $java -Name 'archive_name' `
            -Label 'JRE archive descriptor archive_name'
    ) -Label 'JRE archive'
    $jreArchiveSize = Assert-ZeusDescriptorInteger -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $java -Name 'archive_size' `
            -Label 'JRE archive descriptor size'
    ) -Minimum 1 -Maximum $script:MaximumArchiveBytes -Label 'JRE archive descriptor size'
    $jreArchiveSha256 = Assert-ZeusDescriptorSha256 -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $java -Name 'archive_sha256' `
            -Label 'JRE archive descriptor SHA-256'
    ) -Label 'JRE archive'
    $jreSource = Assert-ZeusDescriptorText -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $java -Name 'source' `
            -Label 'JRE archive source'
    ) -Label 'JRE archive source'
    Get-ZeusDownloadUri -Source $jreSource -ArchiveName $jreArchiveName `
        -Label 'JRE archive' | Out-Null
    $manifestRelative = Assert-ZeusRelativePathText -RelativePath (
        Get-ZeusRequiredDescriptorProperty -InputObject $java -Name 'tree_manifest' `
            -Label 'JRE manifest'
    ) -Label 'JRE manifest'
    $treeFileCount = Assert-ZeusDescriptorInteger -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $java -Name 'tree_file_count' `
            -Label 'JRE manifest file count'
    ) -Minimum 1 -Maximum $script:MaximumJreFiles -Label 'JRE manifest file count'
    $manifestSha256 = Assert-ZeusDescriptorSha256 -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $java -Name 'tree_manifest_sha256' `
            -Label 'JRE manifest descriptor SHA-256'
    ) -Label 'JRE manifest'

    $microemulator = Get-ZeusRequiredDescriptorProperty -InputObject $Descriptor `
        -Name 'microemulator' -Label 'Runtime descriptor microemulator'
    $microemulatorArchiveName = Assert-ZeusArchiveName -ArchiveName (
        Get-ZeusRequiredDescriptorProperty -InputObject $microemulator -Name 'archive_name' `
            -Label 'MicroEmulator archive descriptor archive_name'
    ) -Label 'MicroEmulator archive'
    $microemulatorArchiveSize = Assert-ZeusDescriptorInteger -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $microemulator -Name 'archive_size' `
            -Label 'MicroEmulator archive descriptor size'
    ) -Minimum 1 -Maximum $script:MaximumArchiveBytes `
        -Label 'MicroEmulator archive descriptor size'
    $microemulatorArchiveSha256 = Assert-ZeusDescriptorSha256 -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $microemulator -Name 'archive_sha256' `
            -Label 'MicroEmulator archive descriptor SHA-256'
    ) -Label 'MicroEmulator archive'
    $microemulatorSource = Assert-ZeusDescriptorText -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $microemulator -Name 'source' `
            -Label 'MicroEmulator archive source'
    ) -Label 'MicroEmulator archive source'
    Get-ZeusDownloadUri -Source $microemulatorSource `
        -ArchiveName $microemulatorArchiveName -Label 'MicroEmulator archive' | Out-Null
    $microemulatorJar = Assert-ZeusRelativePathText -RelativePath (
        Get-ZeusRequiredDescriptorProperty -InputObject $microemulator -Name 'jar' `
            -Label 'MicroEmulator JAR'
    ) -Label 'MicroEmulator JAR'
    $microemulatorJarSize = Assert-ZeusDescriptorInteger -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $microemulator -Name 'jar_size' `
            -Label 'MicroEmulator JAR descriptor size'
    ) -Minimum 1 -Maximum $script:MaximumArchiveBytes `
        -Label 'MicroEmulator JAR descriptor size'
    $microemulatorJarSha256 = Assert-ZeusDescriptorSha256 -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $microemulator -Name 'jar_sha256' `
            -Label 'MicroEmulator JAR descriptor SHA-256'
    ) -Label 'MicroEmulator JAR'

    $game = Get-ZeusRequiredDescriptorProperty -InputObject $Descriptor `
        -Name 'game' -Label 'Runtime descriptor game'
    $gameJar = Assert-ZeusRelativePathText -RelativePath (
        Get-ZeusRequiredDescriptorProperty -InputObject $game -Name 'jar' `
            -Label 'Game JAR'
    ) -Label 'Game JAR'
    $gameJarSize = Assert-ZeusDescriptorInteger -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $game -Name 'jar_size' `
            -Label 'Game JAR descriptor size'
    ) -Minimum 1 -Maximum $script:MaximumArchiveBytes -Label 'Game JAR descriptor size'
    $gameJarSha256 = Assert-ZeusDescriptorSha256 -Value (
        Get-ZeusRequiredDescriptorProperty -InputObject $game -Name 'jar_sha256' `
            -Label 'Game JAR descriptor SHA-256'
    ) -Label 'Game JAR'

    if ($jreArchiveName.Equals($microemulatorArchiveName, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'JRE and MicroEmulator archive names must be distinct.'
    }
    foreach ($payloadPath in @($microemulatorJar, $gameJar)) {
        if ($payloadPath.Equals('jre', [StringComparison]::OrdinalIgnoreCase) -or
            $payloadPath.StartsWith('jre/', [StringComparison]::OrdinalIgnoreCase)) {
            throw 'MicroEmulator and game JAR paths must be outside the JRE tree.'
        }
    }
    if ($microemulatorJar.Equals($gameJar, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'MicroEmulator and game JAR paths must be distinct.'
    }

    [pscustomobject]@{
        manifest_path = Resolve-ZeusRelativePath -Root $Root `
            -RelativePath $manifestRelative -Label 'JRE manifest'
        jre_archive_size = $jreArchiveSize
        jre_archive_sha256 = $jreArchiveSha256
        tree_file_count = $treeFileCount
        manifest_sha256 = $manifestSha256
        microemulator_archive_size = $microemulatorArchiveSize
        microemulator_archive_sha256 = $microemulatorArchiveSha256
        microemulator_jar_size = $microemulatorJarSize
        microemulator_jar_sha256 = $microemulatorJarSha256
        game_jar_size = $gameJarSize
        game_jar_sha256 = $gameJarSha256
    }
}

function Open-ZeusRuntimeBootstrap {
    param([Parameter(Mandatory)] [string] $RuntimeRoot)

    $root = Normalize-ZeusDirectoryPath -Path $RuntimeRoot
    Assert-ZeusLocalPlainPath -Path $root -Label 'runtime root'
    Assert-ZeusPlainDirectory -Path $root -Label 'runtime root'
    $descriptorPath = Join-Path $root 'runtime-descriptor.json'
    Assert-ZeusLocalPlainPath -Path $descriptorPath -Label 'runtime descriptor'
    Assert-ZeusPlainFile -Path $descriptorPath -Label 'runtime descriptor'
    $descriptorStream = [IO.File]::Open(
        $descriptorPath,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::Read
    )
    $manifestStream = $null
    try {
        $descriptorBytes = Read-ZeusHeldStreamBytes -Stream $descriptorStream `
            -MaximumBytes $script:MaximumDescriptorBytes -Label 'runtime descriptor'
        try {
            $descriptor = [Text.Encoding]::UTF8.GetString($descriptorBytes) | ConvertFrom-Json -Depth 16
        }
        catch {
            throw "Runtime descriptor JSON is invalid: $($_.Exception.Message)"
        }
        $validatedDescriptor = Assert-ZeusRuntimeDescriptor -Descriptor $descriptor -Root $root
        $manifestPath = $validatedDescriptor.manifest_path
        Assert-ZeusPlainFile -Path $manifestPath -Label 'JRE manifest'
        $manifestStream = [IO.File]::Open(
            $manifestPath,
            [IO.FileMode]::Open,
            [IO.FileAccess]::Read,
            [IO.FileShare]::Read
        )
        $manifestBytes = Read-ZeusHeldStreamBytes -Stream $manifestStream `
            -MaximumBytes $script:MaximumManifestBytes -Label 'JRE manifest'
        $manifestSha256 = [Convert]::ToHexString(
            [Security.Cryptography.SHA256]::HashData($manifestBytes)
        ).ToLowerInvariant()
        if ($validatedDescriptor.manifest_sha256 -ne $manifestSha256) {
            throw 'JRE manifest checksum does not match the runtime descriptor.'
        }
        $manifestText = [Text.Encoding]::UTF8.GetString($manifestBytes)
        ConvertFrom-ZeusJreManifestText -Text $manifestText -Descriptor $descriptor | Out-Null
        [pscustomobject]@{
            root = $root
            descriptor_path = $descriptorPath
            manifest_path = $manifestPath
            descriptor_stream = $descriptorStream
            manifest_stream = $manifestStream
            descriptor_sha256 = [Convert]::ToHexString(
                [Security.Cryptography.SHA256]::HashData($descriptorBytes)
            ).ToLowerInvariant()
            manifest_sha256 = $manifestSha256
        }
    }
    catch {
        if ($null -ne $manifestStream) { $manifestStream.Dispose() }
        $descriptorStream.Dispose()
        throw
    }
}

function Close-ZeusRuntimeBootstrap {
    param([Parameter(Mandatory)] $Bootstrap)

    $Bootstrap.manifest_stream.Dispose()
    $Bootstrap.descriptor_stream.Dispose()
}

function Assert-ZeusRuntimeBootstrapUnchanged {
    param(
        [Parameter(Mandatory)] $Bootstrap,
        [Parameter(Mandatory)] $Context
    )

    if ((Get-ZeusSha256 -Path $Context.descriptor_path) -ne $Bootstrap.descriptor_sha256 -or
        (Get-ZeusSha256 -Path $Context.manifest_path) -ne $Bootstrap.manifest_sha256) {
        throw 'Runtime descriptor or manifest changed while the security boundary was established.'
    }
}

function Read-ZeusRuntimeContext {
    param([Parameter(Mandatory)] [string] $RuntimeRoot)

    $root = Normalize-ZeusDirectoryPath -Path $RuntimeRoot
    Assert-ZeusLocalPlainPath -Path $root -Label 'runtime root'
    Assert-ZeusPlainDirectory -Path $root -Label 'runtime root'
    Assert-ZeusExactAcl -Path $root -Label 'runtime security' -RequireProtected
    $descriptorPath = Join-Path $root 'runtime-descriptor.json'
    Assert-ZeusLocalPlainPath -Path $descriptorPath -Label 'runtime descriptor'
    Assert-ZeusPlainFile -Path $descriptorPath -Label 'runtime descriptor'
    Assert-ZeusSecurePathAcl -Root $root -Path $descriptorPath -Label 'runtime security'
    if ((Get-Item -LiteralPath $descriptorPath).Length -gt $script:MaximumDescriptorBytes) {
        throw 'Runtime descriptor exceeds the 64 KiB limit.'
    }
    try {
        $descriptor = [IO.File]::ReadAllText($descriptorPath) | ConvertFrom-Json -Depth 16
    }
    catch {
        throw "Runtime descriptor JSON is invalid: $($_.Exception.Message)"
    }
    $validatedDescriptor = Assert-ZeusRuntimeDescriptor -Descriptor $descriptor -Root $root
    $manifestPath = $validatedDescriptor.manifest_path
    Assert-ZeusPlainFile -Path $manifestPath -Label 'JRE manifest'
    Assert-ZeusSecurePathAcl -Root $root -Path $manifestPath -Label 'runtime security'
    [pscustomobject]@{
        root = $root
        descriptor_path = $descriptorPath
        manifest_path = $manifestPath
        descriptor = $descriptor
    }
}

function Test-ZeusRuntimePayload {
    param([Parameter(Mandatory)] $Context)

    Assert-ZeusExactAcl -Path $Context.root -Label 'runtime security' -RequireProtected
    $manifest = Read-ZeusJreManifest -Context $Context
    $jreRoot = Join-Path $Context.root 'jre'
    Assert-ZeusPlainDirectory -Path $jreRoot -Label 'JRE root'
    Assert-ZeusExactAcl -Path $jreRoot -Label 'runtime security'
    $observedFiles = [Collections.Generic.Dictionary[string, string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    $directories = [Collections.Generic.Stack[object]]::new()
    $directories.Push([pscustomobject]@{ path = $jreRoot; depth = 0 })
    $observedEntries = 0
    $observedDirectories = 0
    $observedBytes = [int64] 0
    while ($directories.Count -gt 0) {
        $directory = $directories.Pop()
        foreach ($entryPath in [IO.Directory]::EnumerateFileSystemEntries([string] $directory.path)) {
            $observedEntries++
            if ($observedEntries -gt $script:MaximumJreTreeEntries) {
                throw 'JRE tree exceeds the entry-count limit.'
            }
            $entryDepth = [int] $directory.depth + 1
            if ($entryDepth -gt $script:MaximumJreTreeDepth) {
                throw 'JRE tree exceeds the depth limit.'
            }
            $item = Get-Item -LiteralPath $entryPath -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "JRE tree contains a reparse point: $($item.FullName)"
            }
            Assert-ZeusExactAcl -Path $item.FullName -Label 'runtime security'
            if ($item.PSIsContainer) {
                $observedDirectories++
                if ($observedDirectories -gt $script:MaximumJreFiles) {
                    throw 'JRE tree exceeds the directory-count limit.'
                }
                $directories.Push([pscustomobject]@{ path = $item.FullName; depth = $entryDepth })
                continue
            }
            if (-not (Test-Path -LiteralPath $item.FullName -PathType Leaf)) {
                throw "JRE tree contains a non-file entry: $($item.FullName)"
            }
            if ($observedFiles.Count -ge $script:MaximumJreFiles) {
                throw 'JRE tree exceeds the file-count limit.'
            }
            $length = [int64] $item.Length
            if ($length -gt [int64]::MaxValue - $observedBytes) {
                throw 'JRE tree byte total overflowed.'
            }
            $observedBytes += $length
            if ($observedBytes -gt $script:MaximumJreBytes) {
                throw 'JRE tree exceeds the byte limit.'
            }
            $relative = $item.FullName.Substring($jreRoot.Length).TrimStart('\', '/').Replace('\', '/')
            if (-not $observedFiles.TryAdd($relative, $item.FullName)) {
                throw "JRE tree contains a duplicate path: $relative"
            }
        }
    }
    if ($observedFiles.Count -ne $manifest.Count) {
        throw "JRE file set mismatch: expected $($manifest.Count), found $($observedFiles.Count)."
    }
    foreach ($entry in $manifest) {
        if (-not $observedFiles.ContainsKey($entry.relative_path)) {
            throw "JRE manifest entry is missing: $($entry.relative_path)"
        }
        Assert-ZeusPinnedFile -Path $observedFiles[$entry.relative_path] -ExpectedSize $entry.size `
            -ExpectedSha256 $entry.sha256 -Label "JRE file $($entry.relative_path)"
    }

    $microemulatorPath = Resolve-ZeusRelativePath -Root $Context.root `
        -RelativePath ([string] $Context.descriptor.microemulator.jar) -Label 'MicroEmulator JAR'
    Assert-ZeusSecurePathAcl -Root $Context.root -Path $microemulatorPath `
        -Label 'runtime security'
    Assert-ZeusPinnedFile -Path $microemulatorPath `
        -ExpectedSize ([int64] $Context.descriptor.microemulator.jar_size) `
        -ExpectedSha256 ([string] $Context.descriptor.microemulator.jar_sha256) -Label 'MicroEmulator JAR'
    $gamePath = Resolve-ZeusRelativePath -Root $Context.root `
        -RelativePath ([string] $Context.descriptor.game.jar) -Label 'Game JAR'
    Assert-ZeusSecurePathAcl -Root $Context.root -Path $gamePath -Label 'runtime security'
    Assert-ZeusPinnedFile -Path $gamePath -ExpectedSize ([int64] $Context.descriptor.game.jar_size) `
        -ExpectedSha256 ([string] $Context.descriptor.game.jar_sha256) -Label 'Game JAR'

    [pscustomobject]@{
        runtime_id = [string] $Context.descriptor.runtime_id
        jre_file_count = $manifest.Count
        descriptor_sha256 = Get-ZeusSha256 -Path $Context.descriptor_path
    }
}

function Read-ZeusJreManifest {
    param([Parameter(Mandatory)] $Context)

    $manifestItem = Get-Item -LiteralPath $Context.manifest_path
    if ($manifestItem.Length -gt $script:MaximumManifestBytes) {
        throw 'JRE manifest exceeds the 2 MiB limit.'
    }
    Assert-ZeusPinnedFile -Path $Context.manifest_path -ExpectedSize $manifestItem.Length `
        -ExpectedSha256 ([string] $Context.descriptor.java.tree_manifest_sha256) -Label 'JRE manifest'
    $manifestText = [IO.File]::ReadAllText($Context.manifest_path)
    ConvertFrom-ZeusJreManifestText -Text $manifestText -Descriptor $Context.descriptor
}

function ConvertFrom-ZeusJreManifestText {
    param(
        [Parameter(Mandatory)] [string] $Text,
        [Parameter(Mandatory)] $Descriptor
    )

    $entries = [Collections.Generic.List[object]]::new()
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $totalBytes = [int64] 0
    $reader = [IO.StringReader]::new($Text)
    try {
        while ($null -ne ($line = $reader.ReadLine())) {
            $match = [regex]::Match($line, '^([0-9a-f]{64})  ([0-9]+)  (.+)$', [Text.RegularExpressions.RegexOptions]::CultureInvariant)
            if (-not $match.Success) {
                throw 'JRE manifest contains an invalid line.'
            }
            $relative = Assert-ZeusRelativePathText -RelativePath $match.Groups[3].Value -Label 'JRE manifest entry'
            if (-not $seen.Add($relative)) {
                throw "JRE manifest contains a duplicate path: $relative"
            }
            $size = [int64]::Parse($match.Groups[2].Value, [Globalization.CultureInfo]::InvariantCulture)
            if ($size -gt [int64]::MaxValue - $totalBytes) {
                throw 'JRE manifest expanded-size total overflowed.'
            }
            $totalBytes += $size
            if ($totalBytes -gt $script:MaximumJreBytes) {
                throw 'JRE manifest exceeds the 512 MiB expanded-size limit.'
            }
            $entries.Add([pscustomobject]@{
                sha256 = $match.Groups[1].Value
                size = $size
                relative_path = $relative
            })
            if ($entries.Count -gt $script:MaximumJreFiles) {
                throw 'JRE manifest exceeds the 10,000-file limit.'
            }
        }
    }
    finally {
        $reader.Dispose()
    }
    if ($entries.Count -ne [int] $Descriptor.java.tree_file_count) {
        throw "JRE manifest file count mismatch: expected $($Descriptor.java.tree_file_count), found $($entries.Count)."
    }
    $entries.ToArray()
}

function Get-ZeusPinnedArchiveHandle {
    param(
        [string] $OverridePath,
        [Parameter(Mandatory)] [string] $CacheRoot,
        [Parameter(Mandatory)] [string] $ArchiveName,
        [Parameter(Mandatory)] [int64] $ExpectedSize,
        [Parameter(Mandatory)] [string] $ExpectedSha256,
        [Parameter(Mandatory)] [string] $Source,
        [Parameter(Mandatory)] [string] $Label
    )

    if ($ExpectedSize -lt 0 -or $ExpectedSize -gt $script:MaximumArchiveBytes) {
        throw "$Label descriptor size exceeds the archive limit."
    }
    if (-not [string]::IsNullOrWhiteSpace($OverridePath)) {
        $overrideFull = [IO.Path]::GetFullPath($OverridePath)
        Assert-ZeusLocalPlainPath -Path $overrideFull -Label $Label
        return Open-ZeusPinnedReadHandle -Path $overrideFull -ExpectedSize $ExpectedSize `
            -ExpectedSha256 $ExpectedSha256 -Label $Label
    }

    $safeArchiveName = Assert-ZeusArchiveName -ArchiveName $ArchiveName -Label $Label
    $cachePath = Join-Path $CacheRoot $safeArchiveName
    if (Test-Path -LiteralPath $cachePath -PathType Leaf) {
        try {
            Assert-ZeusLocalPlainPath -Path $cachePath -Label $Label
            Assert-ZeusExactAcl -Path $cachePath -Label 'runtime cache security'
            return Open-ZeusPinnedReadHandle -Path $cachePath -ExpectedSize $ExpectedSize `
                -ExpectedSha256 $ExpectedSha256 -Label $Label
        }
        catch {
            Assert-ZeusPlainFile -Path $cachePath -Label $Label
            Remove-Item -LiteralPath $cachePath -Force
        }
    }
    elseif (Test-Path -LiteralPath $cachePath) {
        throw "$Label cache destination is not a plain file: $cachePath"
    }

    $downloadUri = Get-ZeusDownloadUri -Source $Source -ArchiveName $safeArchiveName -Label $Label
    $partialPath = "$cachePath.partial-$([Guid]::NewGuid().ToString('N'))"
    try {
        Invoke-ZeusBoundedDownload -Uri $downloadUri -Destination $partialPath `
            -ExpectedSize $ExpectedSize -Label $Label
        Protect-ZeusPathAcl -Path $partialPath -Label 'runtime cache security'
        $partialHandle = Open-ZeusPinnedReadHandle -Path $partialPath -ExpectedSize $ExpectedSize `
            -ExpectedSha256 $ExpectedSha256 -Label $Label
        $partialHandle.stream.Dispose()
        Move-Item -LiteralPath $partialPath -Destination $cachePath
        Protect-ZeusPathAcl -Path $cachePath -Label 'runtime cache security'
        return Open-ZeusPinnedReadHandle -Path $cachePath -ExpectedSize $ExpectedSize `
            -ExpectedSha256 $ExpectedSha256 -Label $Label
    }
    finally {
        if (Test-Path -LiteralPath $partialPath) {
            Assert-ZeusPlainFile -Path $partialPath -Label "$Label partial download"
            Remove-Item -LiteralPath $partialPath -Force
        }
    }
}

function Read-ZeusStreamBytesAt {
    param(
        [Parameter(Mandatory)] [IO.Stream] $Stream,
        [Parameter(Mandatory)] [int64] $Offset,
        [Parameter(Mandatory)] [int] $Count,
        [Parameter(Mandatory)] [string] $Label
    )

    if (-not $Stream.CanSeek -or $Offset -lt 0 -or $Count -lt 0 -or
        $Offset -gt $Stream.Length - $Count) {
        throw "$Label contains a truncated ZIP structure."
    }
    $originalPosition = $Stream.Position
    $buffer = [byte[]]::new($Count)
    try {
        $Stream.Position = $Offset
        $total = 0
        while ($total -lt $Count) {
            $read = $Stream.Read($buffer, $total, $Count - $total)
            if ($read -eq 0) {
                throw "$Label contains a truncated ZIP structure."
            }
            $total += $read
        }
    }
    finally {
        $Stream.Position = $originalPosition
    }
    return ,$buffer
}

function Get-ZeusZipCentralDirectoryMetadata {
    param(
        [Parameter(Mandatory)] [IO.Stream] $ArchiveStream,
        [Parameter(Mandatory)] [string] $Label
    )

    if (-not $ArchiveStream.CanSeek -or $ArchiveStream.Length -lt 22) {
        throw "$Label is missing a bounded ZIP end record."
    }
    $tailLength = [int] [Math]::Min([int64] 65557, $ArchiveStream.Length)
    $tailOffset = $ArchiveStream.Length - $tailLength
    [byte[]] $tail = Read-ZeusStreamBytesAt -Stream $ArchiveStream `
        -Offset $tailOffset -Count $tailLength -Label $Label
    $eocdIndex = -1
    for ($index = $tail.Length - 22; $index -ge 0; $index--) {
        if ([BitConverter]::ToUInt32($tail, $index) -ne [uint32] 0x06054b50) {
            continue
        }
        $commentLength = [BitConverter]::ToUInt16($tail, $index + 20)
        if ($index + 22 + $commentLength -eq $tail.Length) {
            $eocdIndex = $index
            break
        }
    }
    if ($eocdIndex -lt 0) {
        throw "$Label is missing a bounded ZIP end record."
    }

    $eocdOffset = [uint64] ($tailOffset + $eocdIndex)
    $diskNumber = [BitConverter]::ToUInt16($tail, $eocdIndex + 4)
    $centralDisk = [BitConverter]::ToUInt16($tail, $eocdIndex + 6)
    $entriesOnDisk16 = [BitConverter]::ToUInt16($tail, $eocdIndex + 8)
    $totalEntries16 = [BitConverter]::ToUInt16($tail, $eocdIndex + 10)
    $centralSize32 = [BitConverter]::ToUInt32($tail, $eocdIndex + 12)
    $centralOffset32 = [BitConverter]::ToUInt32($tail, $eocdIndex + 16)
    if ($diskNumber -ne 0 -or $centralDisk -ne 0) {
        throw "$Label uses an unsupported multi-disk ZIP structure."
    }

    $needsZip64 = $entriesOnDisk16 -eq [uint16]::MaxValue -or
        $totalEntries16 -eq [uint16]::MaxValue -or
        $centralSize32 -eq [uint32]::MaxValue -or
        $centralOffset32 -eq [uint32]::MaxValue
    if ($needsZip64) {
        if ($eocdOffset -lt 20) {
            throw "$Label is missing its ZIP64 locator."
        }
        $locatorOffset = [int64] ($eocdOffset - 20)
        [byte[]] $locator = Read-ZeusStreamBytesAt -Stream $ArchiveStream `
            -Offset $locatorOffset -Count 20 -Label $Label
        if ([BitConverter]::ToUInt32($locator, 0) -ne [uint32] 0x07064b50 -or
            [BitConverter]::ToUInt32($locator, 4) -ne 0 -or
            [BitConverter]::ToUInt32($locator, 16) -ne 1) {
            throw "$Label contains an invalid ZIP64 locator."
        }
        $zip64Offset = [BitConverter]::ToUInt64($locator, 8)
        if ($zip64Offset -gt [uint64] [int64]::MaxValue -or
            $zip64Offset -gt [uint64] $locatorOffset -or
            [uint64] $locatorOffset - $zip64Offset -lt 56) {
            throw "$Label contains an invalid ZIP64 end record offset."
        }
        [byte[]] $zip64 = Read-ZeusStreamBytesAt -Stream $ArchiveStream `
            -Offset ([int64] $zip64Offset) -Count 56 -Label $Label
        if ([BitConverter]::ToUInt32($zip64, 0) -ne [uint32] 0x06064b50) {
            throw "$Label contains an invalid ZIP64 end record."
        }
        $zip64RecordSize = [BitConverter]::ToUInt64($zip64, 4)
        if ($zip64RecordSize -lt 44 -or
            $zip64RecordSize -gt [uint64] $locatorOffset - $zip64Offset - 12) {
            throw "$Label contains an invalid ZIP64 end record size."
        }
        if ([BitConverter]::ToUInt32($zip64, 16) -ne 0 -or
            [BitConverter]::ToUInt32($zip64, 20) -ne 0) {
            throw "$Label uses an unsupported multi-disk ZIP64 structure."
        }
        $entriesOnDisk = [BitConverter]::ToUInt64($zip64, 24)
        $totalEntries = [BitConverter]::ToUInt64($zip64, 32)
        $centralSize = [BitConverter]::ToUInt64($zip64, 40)
        $centralOffset = [BitConverter]::ToUInt64($zip64, 48)
        $centralBoundary = $zip64Offset
    }
    else {
        $entriesOnDisk = [uint64] $entriesOnDisk16
        $totalEntries = [uint64] $totalEntries16
        $centralSize = [uint64] $centralSize32
        $centralOffset = [uint64] $centralOffset32
        $centralBoundary = $eocdOffset
    }

    if ($entriesOnDisk -ne $totalEntries) {
        throw "$Label uses an unsupported split central directory."
    }
    if ($totalEntries -gt [uint64] $script:MaximumZipEntries) {
        throw "$Label exceeds the ZIP entry-count limit."
    }
    if ($centralSize -gt [uint64] $script:MaximumZipCentralDirectoryBytes) {
        throw "$Label exceeds the ZIP central-directory byte limit."
    }
    if ($centralOffset -gt [uint64] [int64]::MaxValue -or
        $centralSize -gt [uint64]::MaxValue - $centralOffset) {
        throw "$Label central-directory bounds overflow."
    }
    $centralEnd = $centralOffset + $centralSize
    if ($centralEnd -gt $centralBoundary -or
        $centralEnd -gt [uint64] $ArchiveStream.Length) {
        throw "$Label contains invalid central-directory bounds."
    }

    $cursor = $centralOffset
    for ($entryIndex = [uint64] 0; $entryIndex -lt $totalEntries; $entryIndex++) {
        if ($cursor -gt [uint64] [int64]::MaxValue -or $centralEnd - $cursor -lt 46) {
            throw "$Label central-directory entry is truncated."
        }
        [byte[]] $header = Read-ZeusStreamBytesAt -Stream $ArchiveStream `
            -Offset ([int64] $cursor) -Count 46 -Label $Label
        if ([BitConverter]::ToUInt32($header, 0) -ne [uint32] 0x02014b50) {
            throw "$Label contains an invalid central-directory entry."
        }
        $nameLength = [uint64] [BitConverter]::ToUInt16($header, 28)
        $extraLength = [uint64] [BitConverter]::ToUInt16($header, 30)
        $commentLength = [uint64] [BitConverter]::ToUInt16($header, 32)
        $recordLength = [uint64] 46 + $nameLength + $extraLength + $commentLength
        if ($nameLength -eq 0 -or $recordLength -gt $centralEnd - $cursor) {
            throw "$Label contains invalid central-directory entry bounds."
        }
        $cursor += $recordLength
    }
    if ($cursor -ne $centralEnd) {
        throw "$Label central-directory entry count does not match its bounded byte range."
    }

    [pscustomobject]@{
        entry_count = [int] $totalEntries
        central_directory_size = [int64] $centralSize
    }
}

function Expand-ZeusSafeZip {
    param(
        [Parameter(Mandatory)] [IO.Stream] $ArchiveStream,
        [Parameter(Mandatory)] [string] $Destination,
        [Parameter(Mandatory)] [int64] $MaximumExpandedBytes,
        [Parameter(Mandatory)] [string] $Label
    )

    if (Test-Path -LiteralPath $Destination) {
        throw "$Label staging destination already exists."
    }
    New-ZeusPrivateDirectory -Path $Destination -Label "$Label extraction root"
    $destinationFull = [IO.Path]::GetFullPath($Destination)
    $destinationPrefix = $destinationFull.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    $seenEntries = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $nodes = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $directoryPaths = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    $filePaths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $expandedBytes = [int64] 0
    $centralDirectory = Get-ZeusZipCentralDirectoryMetadata `
        -ArchiveStream $ArchiveStream -Label $Label
    Add-Type -AssemblyName System.IO.Compression
    $ArchiveStream.Position = 0
    $archive = [IO.Compression.ZipArchive]::new(
        $ArchiveStream,
        [IO.Compression.ZipArchiveMode]::Read,
        $true
    )
    try {
        if ($archive.Entries.Count -ne $centralDirectory.entry_count) {
            throw "$Label ZIP entry count changed after bounded central-directory preflight."
        }
        foreach ($entry in $archive.Entries) {
            $entryName = $entry.FullName.Replace('\', '/')
            $trimmed = $entryName.TrimEnd('/')
            if ([string]::IsNullOrWhiteSpace($trimmed) -or $entryName.Contains(':') -or
                [IO.Path]::IsPathRooted($entryName)) {
                throw "$Label contains an unsafe ZIP entry: $entryName"
            }
            $components = @($trimmed.Split('/'))
            if ($components.Count -eq 0 -or @($components | Where-Object {
                [string]::IsNullOrEmpty($_) -or $_ -eq '.' -or $_ -eq '..'
            }).Count -gt 0) {
                throw "$Label contains an unsafe ZIP entry: $entryName"
            }
            if ($components.Count -gt $script:MaximumZipDepth) {
                throw "$Label exceeds the ZIP entry depth limit: $entryName"
            }
            $unixType = ($entry.ExternalAttributes -shr 16) -band 0xF000
            $windowsAttributes = $entry.ExternalAttributes -band 0xFFFF
            if ($unixType -eq 0xA000 -or
                (($windowsAttributes -band [int] [IO.FileAttributes]::ReparsePoint) -ne 0)) {
                throw "$Label contains a linked ZIP entry: $entryName"
            }
            if ($unixType -notin @(0, 0x4000, 0x8000)) {
                throw "$Label contains a special ZIP entry: $entryName"
            }
            if (-not $seenEntries.Add($trimmed)) {
                throw "$Label contains a duplicate ZIP entry: $entryName"
            }
            $relative = $trimmed.Replace('/', [IO.Path]::DirectorySeparatorChar)
            $target = [IO.Path]::GetFullPath((Join-Path $destinationFull $relative))
            if (-not $target.StartsWith($destinationPrefix, [StringComparison]::OrdinalIgnoreCase)) {
                throw "$Label contains an unsafe ZIP entry: $entryName"
            }
            $isDirectory = $entryName.EndsWith('/', [StringComparison]::Ordinal)
            if (($unixType -eq 0x4000) -ne $isDirectory -and $unixType -ne 0) {
                throw "$Label entry type conflicts with its path: $entryName"
            }
            for ($index = 1; $index -lt $components.Count; $index++) {
                $parentPath = ($components[0..($index - 1)] -join '/')
                if ($filePaths.Contains($parentPath)) {
                    throw "$Label file conflicts with an implicit directory: $parentPath"
                }
                $directoryPaths.Add($parentPath) | Out-Null
                $nodes.Add($parentPath) | Out-Null
            }
            if ($isDirectory) {
                if ($filePaths.Contains($trimmed)) {
                    throw "$Label entry conflicts with an existing file: $entryName"
                }
                $directoryPaths.Add($trimmed) | Out-Null
            }
            else {
                if ($directoryPaths.Contains($trimmed)) {
                    throw "$Label entry conflicts with an existing directory: $entryName"
                }
                $filePaths.Add($trimmed) | Out-Null
            }
            $nodes.Add($trimmed) | Out-Null
            if ($nodes.Count -gt $script:MaximumZipNodes) {
                throw "$Label exceeds the ZIP node-count limit."
            }
            if ($isDirectory) {
                New-Item -ItemType Directory -Path $target -Force | Out-Null
                continue
            }
            if ([int64] $entry.Length -gt [int64]::MaxValue - $expandedBytes) {
                throw "$Label expanded-size total overflowed."
            }
            if ([int64] $entry.Length -gt $MaximumExpandedBytes - $expandedBytes) {
                throw "$Label exceeds the expanded-size limit."
            }
            New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
            $input = $entry.Open()
            try {
                $output = [IO.File]::Open($target, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
                try {
                    $written = Copy-ZeusBoundedStream -InputStream $input -OutputStream $output `
                        -MaximumBytes ([int64] $entry.Length) `
                        -ExpectedBytes ([int64] $entry.Length) -Label "$Label entry $entryName"
                    $expandedBytes += $written
                }
                finally {
                    $output.Dispose()
                }
            }
            finally {
                $input.Dispose()
            }
        }
    }
    finally {
        $archive.Dispose()
    }
}

function Copy-ZeusBoundedStream {
    param(
        [Parameter(Mandatory)] [IO.Stream] $InputStream,
        [Parameter(Mandatory)] [IO.Stream] $OutputStream,
        [Parameter(Mandatory)] [int64] $MaximumBytes,
        [Parameter(Mandatory)] [int64] $ExpectedBytes,
        [Parameter(Mandatory)] [string] $Label,
        [Threading.CancellationToken] $CancellationToken = [Threading.CancellationToken]::None
    )

    if ($MaximumBytes -lt 0 -or $ExpectedBytes -lt 0 -or $ExpectedBytes -gt $MaximumBytes) {
        throw "$Label has invalid byte bounds."
    }
    $buffer = [byte[]]::new($script:StreamBufferBytes)
    $total = [int64] 0
    while ($true) {
        $remaining = $MaximumBytes - $total
        $readLimit = if ($remaining -ge $buffer.Length) {
            $buffer.Length
        }
        else {
            [int] $remaining + 1
        }
        $read = if ($CancellationToken.CanBeCanceled) {
            $InputStream.ReadAsync($buffer, 0, $readLimit, $CancellationToken).GetAwaiter().GetResult()
        }
        else {
            $InputStream.Read($buffer, 0, $readLimit)
        }
        if ($read -eq 0) {
            break
        }
        if ($read -gt $remaining) {
            throw "$Label exceeds the actual byte limit of $MaximumBytes."
        }
        if ($CancellationToken.CanBeCanceled) {
            $OutputStream.WriteAsync(
                $buffer,
                0,
                $read,
                $CancellationToken
            ).GetAwaiter().GetResult()
        }
        else {
            $OutputStream.Write($buffer, 0, $read)
        }
        $total += $read
    }
    if ($total -ne $ExpectedBytes) {
        throw "$Label byte count mismatch: expected $ExpectedBytes, found $total."
    }
    $total
}

function Invoke-ZeusBoundedDownload {
    param(
        [Parameter(Mandatory)] [string] $Uri,
        [Parameter(Mandatory)] [string] $Destination,
        [Parameter(Mandatory)] [int64] $ExpectedSize,
        [Parameter(Mandatory)] [string] $Label
    )

    if ($ExpectedSize -lt 0 -or $ExpectedSize -gt $script:MaximumArchiveBytes) {
        throw "$Label descriptor size exceeds the download limit."
    }
    $handler = [Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $true
    $handler.MaxAutomaticRedirections = 10
    $client = [Net.Http.HttpClient]::new($handler, $true)
    $client.Timeout = [Threading.Timeout]::InfiniteTimeSpan
    $deadline = [Threading.CancellationTokenSource]::new([TimeSpan]::FromMinutes(15))
    $response = $null
    $input = $null
    $output = $null
    try {
        $response = $client.GetAsync(
            $Uri,
            [Net.Http.HttpCompletionOption]::ResponseHeadersRead,
            $deadline.Token
        ).GetAwaiter().GetResult()
        $response.EnsureSuccessStatusCode() | Out-Null
        $finalUri = $response.RequestMessage.RequestUri
        if ($finalUri.Scheme -ne [Uri]::UriSchemeHttps -or
            -not [string]::IsNullOrEmpty($finalUri.UserInfo)) {
            throw "$Label redirect target must remain HTTPS without user info."
        }
        $contentLength = $response.Content.Headers.ContentLength
        if ($null -ne $contentLength -and [int64] $contentLength -ne $ExpectedSize) {
            throw "$Label response size mismatch: expected $ExpectedSize, found $contentLength."
        }
        $input = $response.Content.ReadAsStreamAsync($deadline.Token).GetAwaiter().GetResult()
        $output = [IO.File]::Open(
            $Destination,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None
        )
        Copy-ZeusBoundedStream -InputStream $input -OutputStream $output `
            -MaximumBytes $ExpectedSize -ExpectedBytes $ExpectedSize -Label $Label `
            -CancellationToken $deadline.Token | Out-Null
        $output.Flush($true)
    }
    catch {
        if ($deadline.IsCancellationRequested) {
            throw "$Label download exceeded the 15-minute overall deadline."
        }
        throw
    }
    finally {
        if ($null -ne $output) { $output.Dispose() }
        if ($null -ne $input) { $input.Dispose() }
        if ($null -ne $response) { $response.Dispose() }
        $deadline.Dispose()
        $client.Dispose()
    }
}

function Open-ZeusPinnedReadHandle {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [int64] $ExpectedSize,
        [Parameter(Mandatory)] [string] $ExpectedSha256,
        [Parameter(Mandatory)] [string] $Label
    )

    Assert-ZeusPlainFile -Path $Path -Label $Label
    if ($ExpectedSize -lt 0) {
        throw "$Label descriptor size is invalid."
    }
    if ($ExpectedSha256 -notmatch '^[0-9a-f]{64}$') {
        throw "$Label descriptor SHA-256 is invalid."
    }
    $stream = [IO.File]::Open(
        $Path,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::Read
    )
    try {
        if ($stream.Length -ne $ExpectedSize) {
            throw "$Label size mismatch: expected $ExpectedSize, found $($stream.Length)."
        }
        $sha = [Security.Cryptography.SHA256]::Create()
        try {
            $actualSha256 = [Convert]::ToHexString($sha.ComputeHash($stream)).ToLowerInvariant()
        }
        finally {
            $sha.Dispose()
        }
        if ($actualSha256 -ne $ExpectedSha256) {
            throw "$Label checksum mismatch: expected $ExpectedSha256, found $actualSha256."
        }
        $stream.Position = 0
        [pscustomobject]@{ path = $Path; stream = $stream }
    }
    catch {
        $stream.Dispose()
        throw
    }
}

function Copy-ZeusHeldFileToNewPath {
    param(
        [Parameter(Mandatory)] $Handle,
        [Parameter(Mandatory)] [string] $Destination,
        [Parameter(Mandatory)] [int64] $ExpectedSize,
        [Parameter(Mandatory)] [string] $Label
    )

    $Handle.stream.Position = 0
    $output = [IO.File]::Open(
        $Destination,
        [IO.FileMode]::CreateNew,
        [IO.FileAccess]::Write,
        [IO.FileShare]::None
    )
    try {
        Copy-ZeusBoundedStream -InputStream $Handle.stream -OutputStream $output `
            -MaximumBytes $ExpectedSize -ExpectedBytes $ExpectedSize -Label $Label | Out-Null
        $output.Flush($true)
    }
    finally {
        $output.Dispose()
    }
    $Handle.stream.Position = 0
}

function Get-ZeusDirectoryHandleIdentity {
    param(
        [Parameter(Mandatory)] [Microsoft.Win32.SafeHandles.SafeFileHandle] $Handle,
        [Parameter(Mandatory)] [string] $Label
    )

    $information = [ZeusRuntimeNativeSecurity+FileInformation]::new()
    if (-not [ZeusRuntimeNativeSecurity]::GetFileInformationByHandle(
        $Handle,
        [ref] $information
    )) {
        $error = [ComponentModel.Win32Exception]::new(
            [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        )
        throw "$Label identity could not be read: $($error.Message)"
    }
    [pscustomobject]@{
        volume_serial = [uint64] $information.VolumeSerialNumber
        file_index = ([uint64] $information.FileIndexHigh -shl 32) -bor `
            [uint64] $information.FileIndexLow
    }
}

function Get-ZeusFinalDirectoryPath {
    param(
        [Parameter(Mandatory)] [Microsoft.Win32.SafeHandles.SafeFileHandle] $Handle,
        [Parameter(Mandatory)] [string] $Label
    )

    $capacity = [uint32] 32768
    $builder = [Text.StringBuilder]::new([int] $capacity)
    $length = [ZeusRuntimeNativeSecurity]::GetFinalPathNameByHandleW(
        $Handle,
        $builder,
        $capacity,
        [uint32] 0
    )
    if ($length -eq 0 -or $length -ge $capacity) {
        $error = [ComponentModel.Win32Exception]::new(
            [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        )
        throw "$Label canonical path could not be read: $($error.Message)"
    }
    $finalPath = $builder.ToString()
    if ($finalPath.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label canonical path is not on a fixed local drive."
    }
    if ($finalPath.StartsWith('\\?\', [StringComparison]::Ordinal)) {
        $finalPath = $finalPath.Substring(4)
    }
    $normalized = Normalize-ZeusDirectoryPath -Path $finalPath
    $root = [IO.Path]::GetPathRoot($normalized)
    if ($root -notmatch '^[A-Za-z]:\\$') {
        throw "$Label canonical path is not on a fixed local drive."
    }
    $normalized
}

function Open-ZeusPinnedDirectoryHandle {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Label
    )

    $full = Normalize-ZeusDirectoryPath -Path $Path
    Assert-ZeusLocalPlainPath -Path $full -Label $Label
    Assert-ZeusPlainDirectory -Path $full -Label $Label
    $fileReadAttributes = [uint32] 0x80
    $shareReadWriteWithoutDelete = [uint32] 0x3
    $openExisting = [uint32] 3
    $backupSemanticsAndOpenReparsePoint = [uint32] 0x02200000
    $handle = [ZeusRuntimeNativeSecurity]::CreateFileW(
        $full,
        $fileReadAttributes,
        $shareReadWriteWithoutDelete,
        [IntPtr]::Zero,
        $openExisting,
        $backupSemanticsAndOpenReparsePoint,
        [IntPtr]::Zero
    )
    if ($handle.IsInvalid) {
        $error = [ComponentModel.Win32Exception]::new(
            [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        )
        $handle.Dispose()
        throw "$Label could not be pinned against replacement: $($error.Message)"
    }
    try {
        $identity = Get-ZeusDirectoryHandleIdentity -Handle $handle -Label $Label
        $finalPath = Get-ZeusFinalDirectoryPath -Handle $handle -Label $Label
        [pscustomobject]@{
            path = $full
            final_path = $finalPath
            handle = $handle
            volume_serial = $identity.volume_serial
            file_index = $identity.file_index
        }
    }
    catch {
        $handle.Dispose()
        throw
    }
}

function Assert-ZeusPinnedDirectoryIdentity {
    param(
        [Parameter(Mandatory)] $Pin,
        [Parameter(Mandatory)] [string] $Label
    )

    $current = Open-ZeusPinnedDirectoryHandle -Path $Pin.path -Label $Label
    try {
        if ($current.volume_serial -ne $Pin.volume_serial -or
            $current.file_index -ne $Pin.file_index) {
            throw "$Label identity changed during provisioning."
        }
    }
    finally {
        $current.handle.Dispose()
    }
}

function Assert-ZeusPinnedPrivateDirectory {
    param(
        [Parameter(Mandatory)] $Pin,
        [Parameter(Mandatory)] [string] $Label
    )

    Assert-ZeusPinnedDirectoryIdentity -Pin $Pin -Label $Label
    Assert-ZeusExactAcl -Path $Pin.path -Label $Label -RequireProtected
    Assert-ZeusPinnedDirectoryIdentity -Pin $Pin -Label $Label
}

function Normalize-ZeusDirectoryPath {
    param([Parameter(Mandatory)] [string] $Path)

    [IO.Path]::TrimEndingDirectorySeparator([IO.Path]::GetFullPath($Path))
}

function Assert-ZeusCanonicalDirectoriesDisjoint {
    param(
        [Parameter(Mandatory)] [string] $First,
        [Parameter(Mandatory)] [string] $Second,
        [Parameter(Mandatory)] [string] $Label
    )

    $firstPrefix = $First + [IO.Path]::DirectorySeparatorChar
    $secondPrefix = $Second + [IO.Path]::DirectorySeparatorChar
    if ($First.Equals($Second, [StringComparison]::OrdinalIgnoreCase) -or
        $First.StartsWith($secondPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        $Second.StartsWith($firstPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label must be mutually disjoint."
    }
}

function Open-ZeusPinnedDirectoryCandidate {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Label
    )

    $full = Normalize-ZeusDirectoryPath -Path $Path
    Assert-ZeusLocalPlainPath -Path $full -Label $Label
    $missing = [Collections.Generic.Stack[string]]::new()
    $cursor = $full
    while (-not (Test-Path -LiteralPath $cursor)) {
        $component = [IO.Path]::GetFileName($cursor)
        if ([string]::IsNullOrWhiteSpace($component)) {
            throw "$Label has no existing plain ancestor."
        }
        $missing.Push($component)
        $parent = Split-Path -Parent $cursor
        if ([string]::IsNullOrWhiteSpace($parent) -or $parent -eq $cursor) {
            throw "$Label has no existing plain ancestor."
        }
        $cursor = Normalize-ZeusDirectoryPath -Path $parent
    }
    $ancestorPin = Open-ZeusPinnedDirectoryHandle -Path $cursor -Label "$Label existing ancestor"
    try {
        [string[]] $missingComponents = $missing.ToArray()
        $canonical = $ancestorPin.final_path
        foreach ($component in $missingComponents) {
            $canonical = Join-Path $canonical $component
        }
        $canonical = Normalize-ZeusDirectoryPath -Path $canonical
        [pscustomobject]@{
            requested_path = $full
            canonical_path = $canonical
            missing_components = $missingComponents
            ancestor_pin = $ancestorPin
        }
    }
    catch {
        $ancestorPin.handle.Dispose()
        throw
    }
}

function Open-ZeusRuntimeCacheBoundary {
    param(
        [Parameter(Mandatory)] $RuntimePin,
        [Parameter(Mandatory)] [string] $CacheRoot
    )

    Assert-ZeusPinnedDirectoryIdentity -Pin $RuntimePin -Label 'runtime root'
    $candidate = Open-ZeusPinnedDirectoryCandidate -Path $CacheRoot -Label 'runtime cache'
    try {
        Assert-ZeusCanonicalDirectoriesDisjoint -First $RuntimePin.final_path `
            -Second $candidate.canonical_path -Label 'Runtime cache and runtime root'
        if ($candidate.ancestor_pin.volume_serial -ne $RuntimePin.volume_serial) {
            throw 'Runtime cache and runtime root must be on the same fixed local volume.'
        }
        [pscustomobject]@{
            canonical_cache_path = $candidate.canonical_path
            missing_components = $candidate.missing_components
            ancestor_pin = $candidate.ancestor_pin
        }
    }
    catch {
        $candidate.ancestor_pin.handle.Dispose()
        throw
    }
}

function Assert-ZeusLocalPlainPath {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Label
    )

    $full = [IO.Path]::GetFullPath($Path)
    $root = [IO.Path]::GetPathRoot($full)
    if ($root -notmatch '^[A-Za-z]:\\$' -or $full.Substring($root.Length).Contains(':')) {
        throw "$Label must use a fixed local drive-letter path."
    }
    $drive = [IO.DriveInfo]::new($root)
    if ($drive.DriveType -ne [IO.DriveType]::Fixed) {
        throw "$Label must use a fixed local drive."
    }
    $current = $root
    foreach ($component in $full.Substring($root.Length).Split(
        [IO.Path]::DirectorySeparatorChar,
        [StringSplitOptions]::RemoveEmptyEntries
    )) {
        $current = Join-Path $current $component
        if (-not (Test-Path -LiteralPath $current)) {
            break
        }
        $item = Get-Item -LiteralPath $current -Force
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "$Label traverses a reparse point: $current"
        }
    }
}

function New-ZeusExactDirectorySecurity {
    param([switch] $IncludeOwner)

    $currentSid = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $systemSid = [Security.Principal.SecurityIdentifier]::new('S-1-5-18')
    $security = [Security.AccessControl.DirectorySecurity]::new()
    if ($IncludeOwner) {
        $security.SetOwner($currentSid)
    }
    $security.SetAccessRuleProtection($true, $false)
    $inheritance = [Security.AccessControl.InheritanceFlags]::ContainerInherit -bor `
        [Security.AccessControl.InheritanceFlags]::ObjectInherit
    foreach ($sid in @($currentSid, $systemSid)) {
        $rule = [Security.AccessControl.FileSystemAccessRule]::new(
            $sid,
            [Security.AccessControl.FileSystemRights]::FullControl,
            $inheritance,
            [Security.AccessControl.PropagationFlags]::None,
            [Security.AccessControl.AccessControlType]::Allow
        )
        $security.AddAccessRule($rule) | Out-Null
    }
    $security
}

function New-ZeusExactFileSecurity {
    param([switch] $IncludeOwner)

    $currentSid = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $systemSid = [Security.Principal.SecurityIdentifier]::new('S-1-5-18')
    $security = [Security.AccessControl.FileSecurity]::new()
    if ($IncludeOwner) {
        $security.SetOwner($currentSid)
    }
    $security.SetAccessRuleProtection($true, $false)
    foreach ($sid in @($currentSid, $systemSid)) {
        $rule = [Security.AccessControl.FileSystemAccessRule]::new(
            $sid,
            [Security.AccessControl.FileSystemRights]::FullControl,
            [Security.AccessControl.AccessControlType]::Allow
        )
        $security.AddAccessRule($rule) | Out-Null
    }
    $security
}

function New-ZeusPrivateDirectory {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Label
    )

    $full = Normalize-ZeusDirectoryPath -Path $Path
    if (Test-Path -LiteralPath $full) {
        throw "$Label already exists: $full"
    }
    $parent = Split-Path -Parent $full
    Assert-ZeusLocalPlainPath -Path $parent -Label "$Label parent"
    Assert-ZeusPlainDirectory -Path $parent -Label "$Label parent"
    $security = New-ZeusExactDirectorySecurity -IncludeOwner
    $binary = $security.GetSecurityDescriptorBinaryForm()
    $pointer = [Runtime.InteropServices.Marshal]::AllocHGlobal($binary.Length)
    try {
        [Runtime.InteropServices.Marshal]::Copy($binary, 0, $pointer, $binary.Length)
        $attributes = [ZeusRuntimeNativeSecurity+SecurityAttributes]::new()
        $attributes.Length = [Runtime.InteropServices.Marshal]::SizeOf($attributes)
        $attributes.SecurityDescriptor = $pointer
        $attributes.InheritHandle = 0
        if (-not [ZeusRuntimeNativeSecurity]::CreateDirectoryW($full, [ref] $attributes)) {
            $error = [ComponentModel.Win32Exception]::new(
                [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            )
            throw "$Label could not be created privately: $($error.Message)"
        }
    }
    finally {
        [Runtime.InteropServices.Marshal]::FreeHGlobal($pointer)
    }
    Assert-ZeusExactAcl -Path $full -Label "$Label security" -RequireProtected
}

function Test-ZeusDirectoryEmpty {
    param([Parameter(Mandatory)] [string] $Path)

    $enumerator = [IO.Directory]::EnumerateFileSystemEntries($Path).GetEnumerator()
    try {
        -not $enumerator.MoveNext()
    }
    finally {
        $enumerator.Dispose()
    }
}

function Initialize-ZeusExistingPrivateCacheRoot {
    param([Parameter(Mandatory)] [string] $Path)

    $full = Normalize-ZeusDirectoryPath -Path $Path
    Assert-ZeusLocalPlainPath -Path $full -Label 'runtime cache'
    if (-not (Test-Path -LiteralPath $full)) {
        throw "Runtime cache disappeared after its boundary was captured: $full"
    }
    Assert-ZeusPlainDirectory -Path $full -Label 'runtime cache'
    try {
        Assert-ZeusExactAcl -Path $full -Label 'runtime cache security' -RequireProtected
    }
    catch {
        if (-not (Test-ZeusDirectoryEmpty -Path $full)) {
            throw 'Runtime cache has insecure permissions and is not empty; remove its disposable contents before retrying.'
        }
        Protect-ZeusPathAcl -Path $full -Label 'runtime cache security'
        if (-not (Test-ZeusDirectoryEmpty -Path $full)) {
            throw 'Runtime cache changed while its security boundary was being established.'
        }
    }
    Assert-ZeusExactAcl -Path $full -Label 'runtime cache security' -RequireProtected
    $full
}

function Initialize-ZeusPinnedPrivateCacheRoot {
    param([Parameter(Mandatory)] $Boundary)

    $pins = [Collections.Generic.List[object]]::new()
    $success = $false
    try {
        [string[]] $missingComponents = @($Boundary.missing_components)
        if ($missingComponents.Count -eq 0) {
            $cacheFull = Initialize-ZeusExistingPrivateCacheRoot `
                -Path $Boundary.canonical_cache_path
            $cachePin = Open-ZeusPinnedDirectoryHandle -Path $cacheFull `
                -Label 'runtime cache'
            $pins.Add($cachePin)
            Assert-ZeusPinnedPrivateDirectory -Pin $cachePin -Label 'runtime cache security'
            Assert-ZeusPinnedDirectoryIdentity -Pin $Boundary.ancestor_pin `
                -Label 'runtime cache existing ancestor'
        }
        else {
            $currentPath = $Boundary.ancestor_pin.final_path
            $parentPin = $Boundary.ancestor_pin
            foreach ($component in $missingComponents) {
                Assert-ZeusPinnedDirectoryIdentity -Pin $parentPin `
                    -Label 'runtime cache parent'
                $nextPath = Normalize-ZeusDirectoryPath -Path (Join-Path $currentPath $component)
                if (Test-Path -LiteralPath $nextPath) {
                    throw "Runtime cache component appeared concurrently: $nextPath"
                }
                try {
                    New-ZeusPrivateDirectory -Path $nextPath -Label 'runtime cache'
                }
                catch {
                    if (Test-Path -LiteralPath $nextPath) {
                        throw "Runtime cache component appeared concurrently: $nextPath"
                    }
                    throw
                }

                $componentPin = $null
                try {
                    $componentPin = Open-ZeusPinnedDirectoryHandle -Path $nextPath `
                        -Label 'runtime cache'
                    Assert-ZeusPinnedPrivateDirectory -Pin $componentPin `
                        -Label 'runtime cache security'
                    Assert-ZeusPinnedDirectoryIdentity -Pin $parentPin `
                        -Label 'runtime cache parent'
                    if (-not $componentPin.final_path.Equals(
                        $nextPath,
                        [StringComparison]::OrdinalIgnoreCase
                    )) {
                        throw 'Runtime cache component canonical path changed during creation.'
                    }
                    $pins.Add($componentPin)
                    $componentPin = $null
                }
                finally {
                    if ($null -ne $componentPin) {
                        $componentPin.handle.Dispose()
                    }
                }
                $parentPin = $pins[$pins.Count - 1]
                $currentPath = $parentPin.final_path
            }
        }

        $cachePin = $pins[$pins.Count - 1]
        if (-not $cachePin.final_path.Equals(
            $Boundary.canonical_cache_path,
            [StringComparison]::OrdinalIgnoreCase
        )) {
            throw 'Runtime cache canonical path changed while its boundary was established.'
        }
        $parentPins = [Collections.Generic.List[object]]::new()
        for ($index = 0; $index -lt $pins.Count - 1; $index++) {
            $parentPins.Add($pins[$index])
        }
        $result = [pscustomobject]@{
            path = $cachePin.final_path
            cache_pin = $cachePin
            parent_pins = $parentPins.ToArray()
        }
        $success = $true
        $result
    }
    finally {
        if (-not $success) {
            for ($index = $pins.Count - 1; $index -ge 0; $index--) {
                $pins[$index].handle.Dispose()
            }
        }
    }
}

function Close-ZeusPinnedDirectoryChain {
    param(
        [Parameter(Mandatory)] [AllowEmptyCollection()] [object[]] $Pins,
        [Parameter(Mandatory)] [string] $Label
    )

    $failure = $null
    for ($index = $Pins.Count - 1; $index -ge 0; $index--) {
        try {
            Assert-ZeusPinnedDirectoryIdentity -Pin $Pins[$index] -Label $Label
        }
        catch {
            if ($null -eq $failure) {
                $failure = $_
            }
        }
        finally {
            $Pins[$index].handle.Dispose()
        }
    }
    if ($null -ne $failure) {
        throw $failure
    }
}

function Assert-ZeusExactAcl {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Label,
        [switch] $RequireProtected
    )

    $item = Get-Item -LiteralPath $Path -Force
    $sections = [Security.AccessControl.AccessControlSections]::Owner -bor `
        [Security.AccessControl.AccessControlSections]::Access
    $security = if ($item.PSIsContainer) {
        [IO.FileSystemAclExtensions]::GetAccessControl([IO.DirectoryInfo] $item, $sections)
    }
    else {
        [IO.FileSystemAclExtensions]::GetAccessControl([IO.FileInfo] $item, $sections)
    }
    $currentSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $ownerSid = $security.GetOwner([Security.Principal.SecurityIdentifier]).Value
    $rules = @($security.GetAccessRules(
        $true,
        $true,
        [Security.Principal.SecurityIdentifier]
    ))
    $expectedSids = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    $expectedSids.Add($currentSid) | Out-Null
    $expectedSids.Add('S-1-5-18') | Out-Null
    $observedSids = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    $fullControl = [int] [Security.AccessControl.FileSystemRights]::FullControl
    $valid = $ownerSid -eq $currentSid -and `
        (-not $RequireProtected -or $security.AreAccessRulesProtected) -and $rules.Count -eq 2
    foreach ($rule in $rules) {
        $sid = $rule.IdentityReference.Value
        $valid = $valid -and $rule.AccessControlType -eq `
            [Security.AccessControl.AccessControlType]::Allow -and `
            (([int] $rule.FileSystemRights -band $fullControl) -eq $fullControl) -and `
            $expectedSids.Contains($sid) -and $observedSids.Add($sid)
    }
    if (-not $valid -or $observedSids.Count -ne 2) {
        throw "$Label is not protected by exactly the current user and SYSTEM: $Path"
    }
}

function Protect-ZeusPathAcl {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Label
    )

    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Label cannot protect a reparse point: $Path"
    }
    $sections = [Security.AccessControl.AccessControlSections]::Owner
    $existingSecurity = if ($item.PSIsContainer) {
        [IO.FileSystemAclExtensions]::GetAccessControl([IO.DirectoryInfo] $item, $sections)
    }
    else {
        [IO.FileSystemAclExtensions]::GetAccessControl([IO.FileInfo] $item, $sections)
    }
    $currentSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    if ($existingSecurity.GetOwner([Security.Principal.SecurityIdentifier]).Value -ne $currentSid) {
        $ownerOutput = @(& icacls.exe $item.FullName '/setowner' "*$currentSid" '/C' '/Q' 2>&1)
        if ($LASTEXITCODE -ne 0) {
            throw "$Label could not set the current user as owner: $($ownerOutput -join ' ')"
        }
    }
    if ($item.PSIsContainer) {
        [IO.FileSystemAclExtensions]::SetAccessControl(
            [IO.DirectoryInfo] $item,
            (New-ZeusExactDirectorySecurity)
        )
    }
    elseif (Test-Path -LiteralPath $item.FullName -PathType Leaf) {
        [IO.FileSystemAclExtensions]::SetAccessControl(
            [IO.FileInfo] $item,
            (New-ZeusExactFileSecurity)
        )
    }
    else {
        throw "$Label cannot protect a non-file entry: $Path"
    }
    Assert-ZeusExactAcl -Path $item.FullName -Label $Label -RequireProtected
}

function Protect-ZeusAclTree {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Label
    )

    Assert-ZeusLocalPlainPath -Path $Path -Label $Label
    Assert-ZeusPlainDirectory -Path $Path -Label $Label
    Protect-ZeusPathAcl -Path $Path -Label $Label
    $stack = [Collections.Generic.Stack[object]]::new()
    $stack.Push([pscustomobject]@{ path = $Path; depth = 0 })
    $observed = 0
    while ($stack.Count -gt 0) {
        $directory = $stack.Pop()
        foreach ($entryPath in [IO.Directory]::EnumerateFileSystemEntries([string] $directory.path)) {
            $observed++
            if ($observed -gt $script:MaximumRuntimeTreeEntries) {
                throw "$Label exceeds the entry-count limit."
            }
            $depth = [int] $directory.depth + 1
            if ($depth -gt $script:MaximumJreTreeDepth) {
                throw "$Label exceeds the depth limit."
            }
            $item = Get-Item -LiteralPath $entryPath -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "$Label contains a reparse point: $entryPath"
            }
            Protect-ZeusPathAcl -Path $entryPath -Label $Label
            if ($item.PSIsContainer) {
                $stack.Push([pscustomobject]@{ path = $entryPath; depth = $depth })
            }
        }
    }
}

function Assert-ZeusAclTree {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Label
    )

    Assert-ZeusLocalPlainPath -Path $Path -Label $Label
    Assert-ZeusPlainDirectory -Path $Path -Label $Label
    Assert-ZeusExactAcl -Path $Path -Label $Label -RequireProtected
    $stack = [Collections.Generic.Stack[object]]::new()
    $stack.Push([pscustomobject]@{ path = $Path; depth = 0 })
    $observed = 0
    while ($stack.Count -gt 0) {
        $directory = $stack.Pop()
        foreach ($entryPath in [IO.Directory]::EnumerateFileSystemEntries([string] $directory.path)) {
            $observed++
            if ($observed -gt $script:MaximumRuntimeTreeEntries) {
                throw "$Label exceeds the entry-count limit."
            }
            $depth = [int] $directory.depth + 1
            if ($depth -gt $script:MaximumJreTreeDepth) {
                throw "$Label exceeds the depth limit."
            }
            $item = Get-Item -LiteralPath $entryPath -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "$Label contains a reparse point: $entryPath"
            }
            Assert-ZeusExactAcl -Path $entryPath -Label $Label
            if ($item.PSIsContainer) {
                $stack.Push([pscustomobject]@{ path = $entryPath; depth = $depth })
            }
        }
    }
}

function Assert-ZeusSecurePathAcl {
    param(
        [Parameter(Mandatory)] [string] $Root,
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Label
    )

    $rootFull = Normalize-ZeusDirectoryPath -Path $Root
    $pathFull = [IO.Path]::GetFullPath($Path)
    $prefix = $rootFull + [IO.Path]::DirectorySeparatorChar
    if ($pathFull -ne $rootFull -and
        -not $pathFull.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label escaped its security root."
    }
    Assert-ZeusExactAcl -Path $rootFull -Label $Label -RequireProtected
    if ($pathFull -eq $rootFull) {
        return
    }
    $current = $rootFull
    foreach ($component in $pathFull.Substring($prefix.Length).Split(
        [IO.Path]::DirectorySeparatorChar,
        [StringSplitOptions]::RemoveEmptyEntries
    )) {
        $current = Join-Path $current $component
        Assert-ZeusExactAcl -Path $current -Label $Label
    }
}

function Install-ZeusStagedPayload {
    param(
        [Parameter(Mandatory)] [string] $StagedRoot,
        [Parameter(Mandatory)] [string] $RuntimeRoot
    )

    $names = @('jre', 'microemulator', 'game')
    foreach ($name in $names) {
        if (Test-Path -LiteralPath (Join-Path $RuntimeRoot $name)) {
            throw "Runtime payload destination already exists: $name"
        }
    }
    $moved = [Collections.Generic.List[string]]::new()
    try {
        foreach ($name in $names) {
            Move-Item -LiteralPath (Join-Path $StagedRoot $name) -Destination (Join-Path $RuntimeRoot $name)
            $moved.Add($name)
        }
    }
    catch {
        for ($index = $moved.Count - 1; $index -ge 0; $index--) {
            $name = $moved[$index]
            $installed = Join-Path $RuntimeRoot $name
            $staged = Join-Path $StagedRoot $name
            if ((Test-Path -LiteralPath $installed) -and -not (Test-Path -LiteralPath $staged)) {
                Move-Item -LiteralPath $installed -Destination $staged
            }
        }
        throw
    }
}

function Assert-ZeusPinnedFile {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [int64] $ExpectedSize,
        [Parameter(Mandatory)] [string] $ExpectedSha256,
        [Parameter(Mandatory)] [string] $Label
    )

    Assert-ZeusPlainFile -Path $Path -Label $Label
    $actualSize = (Get-Item -LiteralPath $Path).Length
    if ($actualSize -ne $ExpectedSize) {
        throw "$Label size mismatch: expected $ExpectedSize, found $actualSize."
    }
    if ($ExpectedSha256 -notmatch '^[0-9a-f]{64}$') {
        throw "$Label descriptor SHA-256 is invalid."
    }
    $actualSha256 = Get-ZeusSha256 -Path $Path
    if ($actualSha256 -ne $ExpectedSha256) {
        throw "$Label checksum mismatch: expected $ExpectedSha256, found $actualSha256."
    }
}

function Assert-ZeusPlainFile {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Label
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label is missing: $Path"
    }
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Label cannot be a reparse point: $Path"
    }
}

function Assert-ZeusPlainDirectory {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Label
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "$Label is missing: $Path"
    }
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Label cannot be a reparse point: $Path"
    }
}

function Resolve-ZeusRelativePath {
    param(
        [Parameter(Mandatory)] [string] $Root,
        [Parameter(Mandatory)] [string] $RelativePath,
        [Parameter(Mandatory)] [string] $Label
    )

    $normalized = Assert-ZeusRelativePathText -RelativePath $RelativePath -Label $Label
    $rootFull = [IO.Path]::GetFullPath($Root)
    $rootPrefix = $rootFull.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    $joined = [IO.Path]::GetFullPath((Join-Path $rootFull $normalized.Replace('/', [IO.Path]::DirectorySeparatorChar)))
    if (-not $joined.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label escaped the runtime root."
    }
    Assert-ZeusLocalPlainPath -Path $joined -Label $Label
    $joined
}

function Assert-ZeusRelativePathText {
    param(
        [Parameter(Mandatory)] [string] $RelativePath,
        [Parameter(Mandatory)] [string] $Label
    )

    if ([string]::IsNullOrWhiteSpace($RelativePath) -or $RelativePath.Contains('\') -or
        $RelativePath.Contains(':') -or [IO.Path]::IsPathRooted($RelativePath)) {
        throw "$Label path is not normalized and relative: $RelativePath"
    }
    $components = @($RelativePath.Split('/'))
    if (@($components | Where-Object {
        [string]::IsNullOrEmpty($_) -or $_ -eq '.' -or $_ -eq '..'
    }).Count -gt 0) {
        throw "$Label path is not normalized and relative: $RelativePath"
    }
    $RelativePath
}

function Assert-ZeusArchiveName {
    param(
        [Parameter(Mandatory)] [string] $ArchiveName,
        [Parameter(Mandatory)] [string] $Label
    )

    if ([string]::IsNullOrWhiteSpace($ArchiveName) -or
        [IO.Path]::GetFileName($ArchiveName) -ne $ArchiveName -or
        -not $ArchiveName.EndsWith('.zip', [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label descriptor archive_name is invalid."
    }
    $ArchiveName
}

function Get-ZeusDownloadUri {
    param(
        [Parameter(Mandatory)] [string] $Source,
        [Parameter(Mandatory)] [string] $ArchiveName,
        [Parameter(Mandatory)] [string] $Label
    )

    $sourceUri = $null
    if (-not [Uri]::TryCreate($Source, [UriKind]::Absolute, [ref] $sourceUri) -or
        $sourceUri.Scheme -ne [Uri]::UriSchemeHttps -or
        -not [string]::IsNullOrEmpty($sourceUri.UserInfo)) {
        throw "$Label source must be an absolute HTTPS URL without user info."
    }
    if ($sourceUri.AbsolutePath.EndsWith('/' + $ArchiveName, [StringComparison]::OrdinalIgnoreCase) -or
        $sourceUri.AbsolutePath.EndsWith('/download', [StringComparison]::OrdinalIgnoreCase)) {
        return $sourceUri.AbsoluteUri
    }
    ($sourceUri.AbsoluteUri.TrimEnd('/') + '/' + [Uri]::EscapeDataString($ArchiveName) + '/download')
}

function Get-ZeusSha256 {
    param([Parameter(Mandatory)] [string] $Path)

    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function New-ZeusProvisioningResult {
    param(
        [Parameter(Mandatory)] [string] $Status,
        [Parameter(Mandatory)] $Verified
    )

    [pscustomobject][ordered]@{
        schema_version = 1
        status = $Status
        runtime_id = $Verified.runtime_id
        jre_file_count = $Verified.jre_file_count
        descriptor_sha256 = $Verified.descriptor_sha256
    }
}

function Remove-ZeusStagingDirectory {
    param(
        [Parameter(Mandatory)] [string] $StagingRoot,
        [Parameter(Mandatory)] [string] $CacheRoot
    )

    $stagingFull = Normalize-ZeusDirectoryPath -Path $StagingRoot
    $cacheFull = Normalize-ZeusDirectoryPath -Path $CacheRoot
    $stagingParent = Normalize-ZeusDirectoryPath -Path (Split-Path -Parent $stagingFull)
    if (-not $stagingParent.Equals($cacheFull, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($stagingFull) -notlike '.staging-*') {
        throw "Refusing to clean unexpected staging directory: $stagingFull"
    }
    if (Test-Path -LiteralPath $stagingFull) {
        Assert-ZeusLocalPlainPath -Path $cacheFull -Label 'runtime cache cleanup root'
        Assert-ZeusExactAcl -Path $cacheFull -Label 'runtime cache cleanup security' `
            -RequireProtected
        Assert-ZeusPlainDirectory -Path $stagingFull -Label 'runtime staging cleanup root'
        Assert-ZeusExactAcl -Path $stagingFull -Label 'runtime staging cleanup security' `
            -RequireProtected
        $stack = [Collections.Generic.Stack[object]]::new()
        $stack.Push([pscustomobject]@{ path = $stagingFull; depth = 0 })
        $observed = 0
        while ($stack.Count -gt 0) {
            $directory = $stack.Pop()
            foreach ($entryPath in [IO.Directory]::EnumerateFileSystemEntries([string] $directory.path)) {
                $observed++
                if ($observed -gt $script:MaximumStagingTreeEntries) {
                    throw 'Runtime staging cleanup exceeds the entry-count limit.'
                }
                $depth = [int] $directory.depth + 1
                if ($depth -gt $script:MaximumStagingTreeDepth) {
                    throw 'Runtime staging cleanup exceeds the depth limit.'
                }
                $item = Get-Item -LiteralPath $entryPath -Force
                if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                    throw "Runtime staging cleanup contains a reparse point: $entryPath"
                }
                Assert-ZeusExactAcl -Path $entryPath `
                    -Label 'runtime staging cleanup security'
                if ($item.PSIsContainer) {
                    $stack.Push([pscustomobject]@{ path = $entryPath; depth = $depth })
                }
            }
        }
        Remove-Item -LiteralPath $stagingFull -Recurse -Force
    }
}

<#
.SYNOPSIS
Re-applies the pinned runtime's owner-only ACLs to a copied tree.
.DESCRIPTION
A plain `Copy-Item` inherits the destination's ACLs, so a copied runtime is reachable by
Authenticated Users and the application rejects it as insecure. This re-protects every entry with the
same exact current-user/SYSTEM allowlist the provisioner applies, then re-asserts it, so the packaged
runtime satisfies the same check as a freshly provisioned one.
#>
function Protect-ZeusRuntimeCopy {
    param(
        [Parameter(Mandatory)] [string] $Path
    )

    Protect-ZeusAclTree -Path $Path -Label 'packaged runtime security'
    Assert-ZeusAclTree -Path $Path -Label 'packaged runtime security'
}

Export-ModuleMember -Function Invoke-ZeusRuntimeProvisioning, Protect-ZeusRuntimeCopy
