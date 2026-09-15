$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$provisionerPath = Join-Path $PSScriptRoot '..\scripts\Provision-ExactRuntime.ps1'
if (-not (Test-Path -LiteralPath $provisionerPath -PathType Leaf)) {
    throw "RED: runtime provisioner is missing: $provisionerPath"
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

function Assert-ThrowsLike {
    param(
        [Parameter(Mandatory)] [scriptblock] $Action,
        [Parameter(Mandatory)] [string] $Pattern,
        [Parameter(Mandatory)] [string] $Because
    )

    try {
        & $Action
    }
    catch {
        if ($_.Exception.Message -notlike $Pattern) {
            throw "$Because. Expected error like '$Pattern', got '$($_.Exception.Message)'."
        }
        return
    }

    throw "$Because. Expected an error like '$Pattern', but the command succeeded."
}

function Get-Sha256Bytes {
    param([Parameter(Mandatory)] [byte[]] $Bytes)

    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($Bytes)).ToLowerInvariant()
}

function Get-Sha256File {
    param([Parameter(Mandatory)] [string] $Path)

    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Add-AuthenticatedUsersFullControl {
    param([Parameter(Mandatory)] [string] $Path)

    $item = Get-Item -LiteralPath $Path -Force
    $sections = [Security.AccessControl.AccessControlSections]::Access
    $acl = if ($item.PSIsContainer) {
        [IO.FileSystemAclExtensions]::GetAccessControl([IO.DirectoryInfo] $item, $sections)
    }
    else {
        [IO.FileSystemAclExtensions]::GetAccessControl([IO.FileInfo] $item, $sections)
    }
    $authenticatedUsers = [Security.Principal.SecurityIdentifier]::new('S-1-5-11')
    $rule = if ($item.PSIsContainer) {
        [Security.AccessControl.FileSystemAccessRule]::new(
            $authenticatedUsers,
            [Security.AccessControl.FileSystemRights]::FullControl,
            [Security.AccessControl.InheritanceFlags]::ContainerInherit -bor `
                [Security.AccessControl.InheritanceFlags]::ObjectInherit,
            [Security.AccessControl.PropagationFlags]::None,
            [Security.AccessControl.AccessControlType]::Allow
        )
    }
    else {
        [Security.AccessControl.FileSystemAccessRule]::new(
            $authenticatedUsers,
            [Security.AccessControl.FileSystemRights]::FullControl,
            [Security.AccessControl.AccessControlType]::Allow
        )
    }
    $acl.AddAccessRule($rule) | Out-Null
    if ($item.PSIsContainer) {
        [IO.FileSystemAclExtensions]::SetAccessControl([IO.DirectoryInfo] $item, $acl)
    }
    else {
        [IO.FileSystemAclExtensions]::SetAccessControl([IO.FileInfo] $item, $acl)
    }
}

function Get-OwnerAccessSddl {
    param([Parameter(Mandatory)] [string] $Path)

    $item = Get-Item -LiteralPath $Path -Force
    $sections = [Security.AccessControl.AccessControlSections]::Owner -bor `
        [Security.AccessControl.AccessControlSections]::Access
    $acl = if ($item.PSIsContainer) {
        [IO.FileSystemAclExtensions]::GetAccessControl([IO.DirectoryInfo] $item, $sections)
    }
    else {
        [IO.FileSystemAclExtensions]::GetAccessControl([IO.FileInfo] $item, $sections)
    }
    $acl.GetSecurityDescriptorSddlForm($sections)
}

function New-TestZip {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [Collections.IDictionary] $Entries
    )

    Add-Type -AssemblyName System.IO.Compression
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $archive = [IO.Compression.ZipArchive]::new(
            $stream,
            [IO.Compression.ZipArchiveMode]::Create,
            $true
        )
        try {
            foreach ($entryName in $Entries.Keys) {
                $entry = $archive.CreateEntry([string] $entryName, [IO.Compression.CompressionLevel]::NoCompression)
                $entryStream = $entry.Open()
                try {
                    $bytes = [byte[]] $Entries[$entryName]
                    $entryStream.Write($bytes, 0, $bytes.Length)
                }
                finally {
                    $entryStream.Dispose()
                }
            }
        }
        finally {
            $archive.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function New-FakeZipCountStream {
    param(
        [Parameter(Mandatory)] [uint64] $EntryCount,
        [switch] $Zip64
    )

    $stream = [IO.MemoryStream]::new()
    $writer = [IO.BinaryWriter]::new($stream, [Text.Encoding]::UTF8, $true)
    try {
        if ($Zip64) {
            $writer.Write([uint32] 0x06064b50)
            $writer.Write([uint64] 44)
            $writer.Write([uint16] 45)
            $writer.Write([uint16] 45)
            $writer.Write([uint32] 0)
            $writer.Write([uint32] 0)
            $writer.Write($EntryCount)
            $writer.Write($EntryCount)
            $writer.Write([uint64] 0)
            $writer.Write([uint64] 0)

            $writer.Write([uint32] 0x07064b50)
            $writer.Write([uint32] 0)
            $writer.Write([uint64] 0)
            $writer.Write([uint32] 1)

            $writer.Write([uint32] 0x06054b50)
            $writer.Write([uint16] 0)
            $writer.Write([uint16] 0)
            $writer.Write([uint16] 0xFFFF)
            $writer.Write([uint16] 0xFFFF)
            $writer.Write([uint32]::MaxValue)
            $writer.Write([uint32]::MaxValue)
            $writer.Write([uint16] 0)
        }
        else {
            $writer.Write([uint32] 0x06054b50)
            $writer.Write([uint16] 0)
            $writer.Write([uint16] 0)
            $writer.Write([uint16] $EntryCount)
            $writer.Write([uint16] $EntryCount)
            $writer.Write([uint32] 0)
            $writer.Write([uint32] 0)
            $writer.Write([uint16] 0)
        }
        $writer.Flush()
        $stream.Position = 0
        $stream
    }
    catch {
        $stream.Dispose()
        throw
    }
    finally {
        $writer.Dispose()
    }
}

function Write-FixtureDescriptor {
    param(
        [Parameter(Mandatory)] [string] $RuntimeRoot,
        [Parameter(Mandatory)] [string] $JreArchive,
        [Parameter(Mandatory)] [string] $MicroemulatorArchive,
        [Parameter(Mandatory)] [string] $GameJar,
        [Parameter(Mandatory)] [Collections.IDictionary] $JreFiles
    )

    New-Item -ItemType Directory -Path $RuntimeRoot | Out-Null
    $manifestLines = foreach ($relativePath in ($JreFiles.Keys | Sort-Object)) {
        $bytes = [byte[]] $JreFiles[$relativePath]
        "$(Get-Sha256Bytes -Bytes $bytes)  $($bytes.Length)  $relativePath"
    }
    $manifestText = ($manifestLines -join "`n") + "`n"
    $manifestPath = Join-Path $RuntimeRoot 'jre-files.sha256'
    [IO.File]::WriteAllText($manifestPath, $manifestText, [Text.UTF8Encoding]::new($false))

    $microBytes = [Text.Encoding]::UTF8.GetBytes('fixture-microemulator')
    $gameBytes = [IO.File]::ReadAllBytes($GameJar)
    $descriptor = [ordered]@{
        schema_version = 1
        runtime_id = 'windows-x64_fixture-java11_microemu204_ko402'
        created_at_utc = '2026-08-22T12:04:19.283Z'
        platform = [ordered]@{
            os = 'windows'
            architecture = 'x64'
        }
        java = [ordered]@{
            vendor = 'Fixture Vendor'
            distribution = 'Fixture JRE'
            jvm = 'HotSpot'
            version = '11.0.32+9'
            image_type = 'jre'
            archive_name = [IO.Path]::GetFileName($JreArchive)
            archive_size = (Get-Item -LiteralPath $JreArchive).Length
            archive_sha256 = Get-Sha256File -Path $JreArchive
            source = 'https://example.invalid/fixture-jre.zip'
            tree_manifest = 'jre-files.sha256'
            tree_file_count = $JreFiles.Count
            tree_manifest_sha256 = Get-Sha256File -Path $manifestPath
        }
        microemulator = [ordered]@{
            version = '2.0.4'
            archive_name = [IO.Path]::GetFileName($MicroemulatorArchive)
            archive_size = (Get-Item -LiteralPath $MicroemulatorArchive).Length
            archive_sha256 = Get-Sha256File -Path $MicroemulatorArchive
            source = 'https://example.invalid/microemulator/'
            jar = 'microemulator/microemulator.jar'
            jar_size = $microBytes.Length
            jar_sha256 = Get-Sha256Bytes -Bytes $microBytes
            optional_jars = @()
        }
        game = [ordered]@{
            name = 'KnightOnline'
            bundle = '402'
            midlet_version = '1.8.2'
            profile = 'MIDP-2.0'
            configuration = 'CLDC-1.0'
            jar = 'game/KnightOnline_402.jar'
            jar_size = $gameBytes.Length
            jar_sha256 = Get-Sha256Bytes -Bytes $gameBytes
            source_type = 'local_verified_copy'
        }
        launch_defaults = [ordered]@{
            mode = 'classpath_midlet_main'
            main_class = 'org.microemu.app.Main'
            midlet_class = 'com.silverknight.TemMidlet'
            screen_width = 240
            screen_height = 320
            heap_initial_mib = 16
            heap_max_mib = 128
            gc = 'SerialGC'
            use_perf_data = $false
            rms = 'file'
            quiet = $false
            quit_on_midlet_destroy = $true
        }
        validation = [ordered]@{
            status = 'fixture_static_only'
            evidence = 'evidence.json'
            passed = @('artifact_checksums')
            pending = @('isolation', 'containment', 'resource', 'ubuntu_tuple')
        }
    }
    $descriptorPath = Join-Path $RuntimeRoot 'runtime-descriptor.json'
    [IO.File]::WriteAllText(
        $descriptorPath,
        ($descriptor | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false)
    )
}

$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$testRoot = Join-Path $tempBase ("zeus-runtime-provisioning-tests-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null

try {
    $inputs = Join-Path $testRoot 'inputs'
    New-Item -ItemType Directory -Path $inputs | Out-Null
    $jreFiles = [ordered]@{
        'bin/java.exe' = [Text.Encoding]::UTF8.GetBytes('fixture-java')
        'bin/javaw.exe' = [Text.Encoding]::UTF8.GetBytes('fixture-javaw')
        'release' = [Text.Encoding]::UTF8.GetBytes('JAVA_VERSION="11.0.32"')
    }
    $jreEntries = [ordered]@{}
    foreach ($relativePath in $jreFiles.Keys) {
        $jreEntries["fixture-jre/$relativePath"] = $jreFiles[$relativePath]
    }
    $jreArchive = Join-Path $inputs 'fixture-jre.zip'
    New-TestZip -Path $jreArchive -Entries $jreEntries

    $microemulatorArchive = Join-Path $inputs 'microemulator-2.0.4.zip'
    New-TestZip -Path $microemulatorArchive -Entries ([ordered]@{
        'microemulator-2.0.4/microemulator.jar' = [Text.Encoding]::UTF8.GetBytes('fixture-microemulator')
        'microemulator-2.0.4/README' = [Text.Encoding]::UTF8.GetBytes('fixture readme')
    })
    $gameJar = Join-Path $inputs 'KnightOnline_402.jar'
    [IO.File]::WriteAllBytes($gameJar, [Text.Encoding]::UTF8.GetBytes('fixture-game-402'))

    $unrelatedRoot = Join-Path $testRoot 'unrelated-mistyped-runtime-root'
    New-Item -ItemType Directory -Path $unrelatedRoot | Out-Null
    $unrelatedFile = Join-Path $unrelatedRoot 'operator-data.txt'
    [IO.File]::WriteAllText($unrelatedFile, 'must-not-be-mutated')
    $unrelatedRootBefore = Get-OwnerAccessSddl -Path $unrelatedRoot
    $unrelatedFileBefore = Get-OwnerAccessSddl -Path $unrelatedFile
    Assert-ThrowsLike -Pattern '*runtime descriptor*missing*' `
        -Because 'A mistyped runtime root must fail structural validation' -Action {
            & $provisionerPath -RuntimeRoot $unrelatedRoot `
                -CacheRoot (Join-Path $testRoot 'cache-unrelated-root') -GameJar $gameJar `
                -JreArchive $jreArchive -MicroemulatorArchive $microemulatorArchive
        }
    Assert-Equal (Get-OwnerAccessSddl -Path $unrelatedRoot) $unrelatedRootBefore `
        'A mistyped runtime root must not have its owner or DACL mutated'
    Assert-Equal (Get-OwnerAccessSddl -Path $unrelatedFile) $unrelatedFileBefore `
        'Files below a mistyped runtime root must remain untouched'

    $invalidDescriptorRoot = Join-Path $testRoot 'runtime-invalid-descriptor'
    Write-FixtureDescriptor -RuntimeRoot $invalidDescriptorRoot -JreArchive $jreArchive `
        -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar -JreFiles $jreFiles
    $invalidDescriptorPath = Join-Path $invalidDescriptorRoot 'runtime-descriptor.json'
    $invalidManifestPath = Join-Path $invalidDescriptorRoot 'jre-files.sha256'
    $invalidDescriptor = Get-Content -LiteralPath $invalidDescriptorPath -Raw |
        ConvertFrom-Json -Depth 16
    $invalidDescriptor.game.jar_sha256 = 'invalid'
    [IO.File]::WriteAllText(
        $invalidDescriptorPath,
        ($invalidDescriptor | ConvertTo-Json -Depth 16),
        [Text.UTF8Encoding]::new($false)
    )
    Add-AuthenticatedUsersFullControl -Path $invalidDescriptorRoot
    Add-AuthenticatedUsersFullControl -Path $invalidDescriptorPath
    Add-AuthenticatedUsersFullControl -Path $invalidManifestPath
    $invalidRootBefore = Get-OwnerAccessSddl -Path $invalidDescriptorRoot
    $invalidDescriptorBefore = Get-OwnerAccessSddl -Path $invalidDescriptorPath
    $invalidManifestBefore = Get-OwnerAccessSddl -Path $invalidManifestPath
    Assert-ThrowsLike -Pattern '*Game JAR descriptor SHA-256 is invalid*' `
        -Because 'A malformed held descriptor must fail before hardening the runtime tree' `
        -Action {
            & $provisionerPath -RuntimeRoot $invalidDescriptorRoot `
                -CacheRoot (Join-Path $testRoot 'cache-invalid-descriptor') `
                -GameJar $gameJar -JreArchive $jreArchive `
                -MicroemulatorArchive $microemulatorArchive
        }
    Assert-Equal (Get-OwnerAccessSddl -Path $invalidDescriptorRoot) $invalidRootBefore `
        'A malformed descriptor must not mutate the runtime root owner or DACL'
    Assert-Equal (Get-OwnerAccessSddl -Path $invalidDescriptorPath) $invalidDescriptorBefore `
        'A malformed descriptor must not mutate its own owner or DACL'
    Assert-Equal (Get-OwnerAccessSddl -Path $invalidManifestPath) $invalidManifestBefore `
        'A malformed descriptor must not mutate the manifest owner or DACL'

    $runtimeRoot = Join-Path $testRoot 'runtime-success'
    Write-FixtureDescriptor -RuntimeRoot $runtimeRoot -JreArchive $jreArchive `
        -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar -JreFiles $jreFiles
    $cacheRoot = Join-Path $testRoot 'cache-success'
    $cacheRootWithTrailingSeparator = $cacheRoot + [IO.Path]::DirectorySeparatorChar
    $provisioned = (& $provisionerPath -RuntimeRoot $runtimeRoot `
        -CacheRoot $cacheRootWithTrailingSeparator `
        -JreArchive $jreArchive -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar) |
        ConvertFrom-Json

    Assert-Equal $provisioned.status 'provisioned' 'A complete staged payload must be installed'
    Assert-Equal $provisioned.runtime_id 'windows-x64_fixture-java11_microemu204_ko402' `
        'Provisioning must report the descriptor runtime ID'
    Assert-Equal $provisioned.jre_file_count 3 'Provisioning must verify every manifest entry'
    Assert-Equal ([IO.File]::ReadAllText((Join-Path $runtimeRoot 'jre\bin\java.exe'))) `
        'fixture-java' 'JRE content must be installed below jre/'
    Assert-Equal ([IO.File]::ReadAllText((Join-Path $runtimeRoot 'microemulator\microemulator.jar'))) `
        'fixture-microemulator' 'Only the pinned MicroEmulator JAR must be installed'
    Assert-Equal ([IO.File]::ReadAllText((Join-Path $runtimeRoot 'game\KnightOnline_402.jar'))) `
        'fixture-game-402' 'The supplied game JAR must be copied into the exact descriptor path'
    Assert-Equal @(Get-ChildItem -LiteralPath $cacheRoot -Directory -Filter '.staging-*' -ErrorAction SilentlyContinue).Count `
        0 'Successful provisioning must clean staging directories'

    $verified = (& $provisionerPath -RuntimeRoot $runtimeRoot `
        -CacheRoot $cacheRootWithTrailingSeparator -VerifyOnly) |
        ConvertFrom-Json
    Assert-Equal $verified.status 'verified' 'VerifyOnly must accept an already provisioned runtime'
    Assert-Equal $verified.jre_file_count 3 'VerifyOnly must recheck the complete JRE manifest'

    $alreadyProvisioned = (& $provisionerPath -RuntimeRoot $runtimeRoot `
        -CacheRoot $cacheRootWithTrailingSeparator) |
        ConvertFrom-Json
    Assert-Equal $alreadyProvisioned.status 'already_provisioned' `
        'A second provisioning call must be an idempotent verification without requiring inputs'

    $provisioningModule = Import-Module `
        (Join-Path $PSScriptRoot '..\scripts\Zeus.RuntimeProvisioning.psm1') -Force -PassThru
    $extraEntry = Join-Path $runtimeRoot 'jre\extra-empty-directory'
    New-Item -ItemType Directory -Path $extraEntry | Out-Null
    try {
        Assert-ThrowsLike -Pattern '*JRE tree exceeds the entry-count limit*' `
            -Because 'Verify must stop streaming the observed tree at a bounded entry count' -Action {
                & $provisioningModule {
                    param($FixtureRoot)

                    $previous = $script:MaximumJreTreeEntries
                    try {
                        $script:MaximumJreTreeEntries = 4
                        $fixtureContext = Read-ZeusRuntimeContext -RuntimeRoot $FixtureRoot
                        Test-ZeusRuntimePayload -Context $fixtureContext | Out-Null
                    }
                    finally {
                        $script:MaximumJreTreeEntries = $previous
                    }
                } $runtimeRoot
            }
    }
    finally {
        Remove-Item -LiteralPath $extraEntry -Force
    }

    $deepRoot = Join-Path $runtimeRoot 'jre\deep\one\two'
    New-Item -ItemType Directory -Path $deepRoot -Force | Out-Null
    try {
        Assert-ThrowsLike -Pattern '*JRE tree exceeds the depth limit*' `
            -Because 'Verify must reject path depth before continuing recursive traversal' -Action {
                & $provisioningModule {
                    param($FixtureRoot)

                    $previous = $script:MaximumJreTreeDepth
                    try {
                        $script:MaximumJreTreeDepth = 2
                        $fixtureContext = Read-ZeusRuntimeContext -RuntimeRoot $FixtureRoot
                        Test-ZeusRuntimePayload -Context $fixtureContext | Out-Null
                    }
                    finally {
                        $script:MaximumJreTreeDepth = $previous
                    }
                } $runtimeRoot
            }
    }
    finally {
        Remove-Item -LiteralPath (Join-Path $runtimeRoot 'jre\deep') -Recurse -Force
    }

    Assert-ThrowsLike -Pattern '*fixture stream exceeds the actual byte limit*' `
        -Because 'Stream copies must enforce bytes actually read instead of declared metadata' -Action {
            & $provisioningModule {
                $input = [IO.MemoryStream]::new([byte[]] (1, 2, 3, 4, 5))
                $output = [IO.MemoryStream]::new()
                try {
                    Copy-ZeusBoundedStream -InputStream $input -OutputStream $output `
                        -MaximumBytes 4 -ExpectedBytes 4 -Label 'fixture stream' | Out-Null
                }
                finally {
                    $output.Dispose()
                    $input.Dispose()
                }
            }
        }

    Assert-ThrowsLike -Pattern '*canceled*' `
        -Because 'A canceled overall download deadline must interrupt the body read' -Action {
            & $provisioningModule {
                $input = [IO.MemoryStream]::new([byte[]] (1, 2, 3, 4))
                $output = [IO.MemoryStream]::new()
                $deadline = [Threading.CancellationTokenSource]::new()
                $deadline.Cancel()
                try {
                    Copy-ZeusBoundedStream -InputStream $input -OutputStream $output `
                        -MaximumBytes 4 -ExpectedBytes 4 -Label 'deadline fixture stream' `
                        -CancellationToken $deadline.Token | Out-Null
                }
                finally {
                    $deadline.Dispose()
                    $output.Dispose()
                    $input.Dispose()
                }
            }
        }

    $deepZip = Join-Path $inputs 'deep-entry.zip'
    New-TestZip -Path $deepZip -Entries ([ordered]@{
        'root/one/two/three/file.bin' = [byte[]] (1, 2, 3)
    })
    $deepExtraction = Join-Path $testRoot 'deep-extraction'
    try {
        Assert-ThrowsLike -Pattern '*ZIP entry depth limit*' `
            -Because 'ZIP extraction must reject depth before creating implicit directories' `
            -Action {
                & $provisioningModule {
                    param($ArchivePath, $DestinationPath)

                    $previous = $script:MaximumZipDepth
                    $stream = [IO.File]::OpenRead($ArchivePath)
                    try {
                        $script:MaximumZipDepth = 3
                        Expand-ZeusSafeZip -ArchiveStream $stream -Destination $DestinationPath `
                            -MaximumExpandedBytes 1MB -Label 'deep fixture archive'
                    }
                    finally {
                        $script:MaximumZipDepth = $previous
                        $stream.Dispose()
                    }
                } $deepZip $deepExtraction
            }
    }
    finally {
        if (Test-Path -LiteralPath $deepExtraction) {
            Remove-Item -LiteralPath $deepExtraction -Recurse -Force
        }
    }

    foreach ($zip64 in @($false, $true)) {
        Assert-ThrowsLike -Pattern '*ZIP entry-count limit*' `
            -Because 'ZIP entry counts must be rejected before ZipArchive materializes entries' `
            -Action {
                $fakeArchive = New-FakeZipCountStream -EntryCount 10001 -Zip64:$zip64
                try {
                    & $provisioningModule {
                        param($ArchiveStream)
                        Get-ZeusZipCentralDirectoryMetadata -ArchiveStream $ArchiveStream `
                            -Label 'count fixture archive' | Out-Null
                    } $fakeArchive
                }
                finally {
                    $fakeArchive.Dispose()
                }
            }
    }

    & $provisioningModule {
        if ($script:MaximumStagingTreeEntries -lt
            (2 * $script:MaximumZipNodes + 16)) {
            throw 'Staging cleanup entry budget must cover both maximum-size archives plus payload metadata.'
        }
        if ($script:MaximumStagingTreeDepth -lt ($script:MaximumZipDepth + 2)) {
            throw 'Staging cleanup depth must cover a maximum-depth archive below its extraction root.'
        }
    }

    $pinRoot = Join-Path $testRoot 'pinned-directory-root'
    & $provisioningModule {
        param($DirectoryPath)
        New-ZeusPrivateDirectory -Path $DirectoryPath -Label 'pin fixture root'
    } $pinRoot
    $directoryPin = & $provisioningModule {
        param($DirectoryPath)
        Open-ZeusPinnedDirectoryHandle -Path $DirectoryPath -Label 'pin fixture root'
    } $pinRoot
    try {
        $pinRootMoved = $pinRoot + '-moved'
        $renameSucceeded = $false
        try {
            Move-Item -LiteralPath $pinRoot -Destination $pinRootMoved
            $renameSucceeded = $true
        }
        catch {
            Assert-True (Test-Path -LiteralPath $pinRoot -PathType Container) `
                'A blocked directory rename must leave the pinned root in place'
        }
        if ($renameSucceeded) {
            New-Item -ItemType Directory -Path $pinRoot | Out-Null
            Assert-ThrowsLike -Pattern '*identity changed*' `
                -Because 'A renamed and replaced root must fail pinned identity validation' `
                -Action {
                    & $provisioningModule {
                        param($Pin)
                        Assert-ZeusPinnedDirectoryIdentity -Pin $Pin -Label 'pin fixture root'
                    } $directoryPin
                }
        }
    }
    finally {
        $directoryPin.handle.Dispose()
    }

    $pinnedPrivateRoot = Join-Path $testRoot 'pinned-private-directory-root'
    & $provisioningModule {
        param($DirectoryPath)
        New-ZeusPrivateDirectory -Path $DirectoryPath -Label 'private pin fixture root'
    } $pinnedPrivateRoot
    $privateDirectoryPin = & $provisioningModule {
        param($DirectoryPath)
        Open-ZeusPinnedDirectoryHandle -Path $DirectoryPath -Label 'private pin fixture root'
    } $pinnedPrivateRoot
    try {
        Add-AuthenticatedUsersFullControl -Path $pinnedPrivateRoot
        Assert-ThrowsLike -Pattern '*not protected by exactly the current user and SYSTEM*' `
            -Because 'A pinned cache or staging replacement must have its ACL revalidated' `
            -Action {
                & $provisioningModule {
                    param($Pin)
                    Assert-ZeusPinnedPrivateDirectory -Pin $Pin `
                        -Label 'private pin fixture root'
                } $privateDirectoryPin
            }
    }
    finally {
        $privateDirectoryPin.handle.Dispose()
    }

    $raceCacheAncestor = Join-Path $testRoot 'cache-race-ancestor'
    $raceCacheOutside = Join-Path $testRoot 'cache-race-outside'
    New-Item -ItemType Directory -Path $raceCacheAncestor | Out-Null
    New-Item -ItemType Directory -Path $raceCacheOutside | Out-Null
    $raceOutsideFile = Join-Path $raceCacheOutside 'operator-data.txt'
    [IO.File]::WriteAllText($raceOutsideFile, 'must-not-be-hardened-through-a-new-junction')
    Add-AuthenticatedUsersFullControl -Path $raceCacheOutside
    Add-AuthenticatedUsersFullControl -Path $raceOutsideFile
    $raceOutsideRootBefore = Get-OwnerAccessSddl -Path $raceCacheOutside
    $raceOutsideFileBefore = Get-OwnerAccessSddl -Path $raceOutsideFile
    $raceRuntimePin = & $provisioningModule {
        param($DirectoryPath)
        Open-ZeusPinnedDirectoryHandle -Path $DirectoryPath -Label 'race fixture runtime'
    } $runtimeRoot
    $raceBoundary = $null
    try {
        $raceCacheRoot = Join-Path $raceCacheAncestor 'inserted\cache'
        $raceBoundary = & $provisioningModule {
            param($RuntimePin, $CachePath)
            Open-ZeusRuntimeCacheBoundary -RuntimePin $RuntimePin -CacheRoot $CachePath
        } $raceRuntimePin $raceCacheRoot
        New-Item -ItemType Junction -Path (Join-Path $raceCacheAncestor 'inserted') `
            -Target $raceCacheOutside | Out-Null
        Assert-ThrowsLike -Pattern '*appeared concurrently*' `
            -Because 'A missing cache component inserted after boundary capture must not be adopted' `
            -Action {
                & $provisioningModule {
                    param($Boundary)
                    Initialize-ZeusPinnedPrivateCacheRoot -Boundary $Boundary | Out-Null
                } $raceBoundary
            }
        Assert-Equal (Get-OwnerAccessSddl -Path $raceCacheOutside) $raceOutsideRootBefore `
            'A concurrent cache junction must not mutate its target root owner or DACL'
        Assert-Equal (Get-OwnerAccessSddl -Path $raceOutsideFile) $raceOutsideFileBefore `
            'A concurrent cache junction must not mutate target file owner or DACL'
    }
    finally {
        if ($null -ne $raceBoundary) {
            $raceBoundary.ancestor_pin.handle.Dispose()
        }
        $raceRuntimePin.handle.Dispose()
    }

    $installedGamePath = Join-Path $runtimeRoot 'game\KnightOnline_402.jar'
    Add-AuthenticatedUsersFullControl -Path $installedGamePath
    Assert-ThrowsLike -Pattern '*runtime security*' `
        -Because 'VerifyOnly must reject an explicit broad payload ACE' -Action {
            & $provisionerPath -RuntimeRoot $runtimeRoot -CacheRoot $cacheRoot -VerifyOnly
        }
    $repairedAcl = (& $provisionerPath -RuntimeRoot $runtimeRoot -CacheRoot $cacheRoot) |
        ConvertFrom-Json
    Assert-Equal $repairedAcl.status 'already_provisioned' `
        'Normal idempotent provisioning must replace and verify an insecure explicit DACL'

    $unrelatedRuntimeFile = Join-Path $runtimeRoot 'unrelated-security-probe.txt'
    [IO.File]::WriteAllText($unrelatedRuntimeFile, 'security-probe')
    Add-AuthenticatedUsersFullControl -Path $unrelatedRuntimeFile
    Assert-ThrowsLike -Pattern '*runtime security*' `
        -Because 'VerifyOnly must inspect every runtime entry, not only descriptor payload paths' `
        -Action {
            & $provisionerPath -RuntimeRoot $runtimeRoot -CacheRoot $cacheRoot -VerifyOnly
        }
    $repairedTreeAcl = (& $provisionerPath -RuntimeRoot $runtimeRoot -CacheRoot $cacheRoot) |
        ConvertFrom-Json
    Assert-Equal $repairedTreeAcl.status 'already_provisioned' `
        'Normal idempotent provisioning must repair security on every runtime entry'
    Remove-Item -LiteralPath $unrelatedRuntimeFile -Force

    [IO.File]::WriteAllText(
        (Join-Path $runtimeRoot 'game\KnightOnline_402.jar'),
        'tampered-game',
        [Text.UTF8Encoding]::new($false)
    )
    Assert-ThrowsLike -Pattern '*Game JAR size mismatch*' `
        -Because 'VerifyOnly must reject a modified local game payload' -Action {
            & $provisionerPath -RuntimeRoot $runtimeRoot -CacheRoot $cacheRoot -VerifyOnly
        }
    Assert-ThrowsLike -Pattern '*refusing to overwrite*' `
        -Because 'Normal provisioning must not repair content by overwriting a corrupt payload' -Action {
            & $provisionerPath -RuntimeRoot $runtimeRoot -CacheRoot $cacheRoot `
                -JreArchive $jreArchive -MicroemulatorArchive $microemulatorArchive `
                -GameJar $gameJar
        }

    $missingGameRoot = Join-Path $testRoot 'runtime-missing-game-input'
    Write-FixtureDescriptor -RuntimeRoot $missingGameRoot -JreArchive $jreArchive `
        -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar -JreFiles $jreFiles
    Assert-ThrowsLike -Pattern '*-GameJar*' `
        -Because 'A fresh runtime must require an operator-supplied legal game JAR' -Action {
            & $provisionerPath -RuntimeRoot $missingGameRoot -CacheRoot (Join-Path $testRoot 'cache-missing-game') `
                -JreArchive $jreArchive -MicroemulatorArchive $microemulatorArchive
        }
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $missingGameRoot 'jre'))) `
        'Missing game input must fail before installing any payload directory'

    $corruptJreArchive = Join-Path $inputs 'corrupt-jre.zip'
    New-TestZip -Path $corruptJreArchive -Entries ([ordered]@{
        'fixture-jre/bin/java.exe' = [Text.Encoding]::UTF8.GetBytes('wrong-java')
    })
    $corruptRoot = Join-Path $testRoot 'runtime-corrupt-archive'
    Write-FixtureDescriptor -RuntimeRoot $corruptRoot -JreArchive $jreArchive `
        -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar -JreFiles $jreFiles
    Assert-ThrowsLike -Pattern '*JRE archive size mismatch*' `
        -Because 'An archive that differs from the descriptor must be rejected before extraction' -Action {
            & $provisionerPath -RuntimeRoot $corruptRoot -CacheRoot (Join-Path $testRoot 'cache-corrupt') `
                -JreArchive $corruptJreArchive -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar
        }
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $corruptRoot 'jre'))) `
        'A rejected archive must not leave a partial JRE'

    $traversalArchive = Join-Path $inputs 'traversal-jre.zip'
    $traversalEntries = [ordered]@{}
    foreach ($entryName in $jreEntries.Keys) {
        $traversalEntries[$entryName] = $jreEntries[$entryName]
    }
    $traversalEntries['../escape.txt'] = [Text.Encoding]::UTF8.GetBytes('escape')
    New-TestZip -Path $traversalArchive -Entries $traversalEntries
    $traversalRoot = Join-Path $testRoot 'runtime-traversal-archive'
    Write-FixtureDescriptor -RuntimeRoot $traversalRoot -JreArchive $traversalArchive `
        -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar -JreFiles $jreFiles
    Assert-ThrowsLike -Pattern '*unsafe ZIP entry*' `
        -Because 'A checksum-pinned ZIP must still reject path traversal entries' -Action {
            & $provisionerPath -RuntimeRoot $traversalRoot -CacheRoot (Join-Path $testRoot 'cache-traversal') `
                -JreArchive $traversalArchive -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar
        }
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $testRoot 'escape.txt'))) `
        'Unsafe archive content must never escape the staging directory'

    $insecureCacheRoot = Join-Path $testRoot 'insecure-nonempty-cache'
    New-Item -ItemType Directory -Path $insecureCacheRoot | Out-Null
    Add-AuthenticatedUsersFullControl -Path $insecureCacheRoot
    [IO.File]::WriteAllText((Join-Path $insecureCacheRoot 'untrusted.cache'), 'untrusted')
    $insecureCacheRuntime = Join-Path $testRoot 'runtime-insecure-cache'
    Write-FixtureDescriptor -RuntimeRoot $insecureCacheRuntime -JreArchive $jreArchive `
        -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar -JreFiles $jreFiles
    Assert-ThrowsLike -Pattern '*insecure permissions and is not empty*' `
        -Because 'A pre-existing broad cache with content must never be trusted or consumed' -Action {
            & $provisionerPath -RuntimeRoot $insecureCacheRuntime `
                -CacheRoot $insecureCacheRoot -JreArchive $jreArchive `
                -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar
        }
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $insecureCacheRuntime 'jre'))) `
        'An insecure cache refusal must happen before installing payload content'

    $emptyCacheRoot = Join-Path $testRoot 'insecure-empty-cache'
    New-Item -ItemType Directory -Path $emptyCacheRoot | Out-Null
    Add-AuthenticatedUsersFullControl -Path $emptyCacheRoot
    $emptyCacheRuntime = Join-Path $testRoot 'runtime-empty-cache'
    Write-FixtureDescriptor -RuntimeRoot $emptyCacheRuntime -JreArchive $jreArchive `
        -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar -JreFiles $jreFiles
    $emptyCacheResult = (& $provisionerPath -RuntimeRoot $emptyCacheRuntime `
        -CacheRoot $emptyCacheRoot -JreArchive $jreArchive `
        -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar) | ConvertFrom-Json
    Assert-Equal $emptyCacheResult.status 'provisioned' `
        'An empty cache may be hardened before any archive is consumed'
    & $provisioningModule {
        param($CachePath)
        Assert-ZeusExactAcl -Path $CachePath -Label 'runtime cache security' -RequireProtected
    } $emptyCacheRoot

    $linkedRuntimeTarget = Join-Path $testRoot 'linked-runtime-target'
    $linkedRuntimeRoot = Join-Path $testRoot 'linked-runtime-root'
    Write-FixtureDescriptor -RuntimeRoot $linkedRuntimeTarget -JreArchive $jreArchive `
        -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar -JreFiles $jreFiles
    New-Item -ItemType Junction -Path $linkedRuntimeRoot -Target $linkedRuntimeTarget | Out-Null
    Assert-ThrowsLike -Pattern '*reparse point*' `
        -Because 'A runtime root reached through a junction must fail before ACL or content reads' `
        -Action {
            & $provisionerPath -RuntimeRoot $linkedRuntimeRoot `
                -CacheRoot (Join-Path $testRoot 'cache-linked-runtime') -VerifyOnly
        }

    $overlapRuntimeRoot = Join-Path $testRoot 'runtime-overlapping-cache'
    Write-FixtureDescriptor -RuntimeRoot $overlapRuntimeRoot -JreArchive $jreArchive `
        -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar -JreFiles $jreFiles
    Assert-ThrowsLike -Pattern '*mutually disjoint*' `
        -Because 'A cache below the runtime root must fail before creating a payload directory' `
        -Action {
            & $provisionerPath -RuntimeRoot $overlapRuntimeRoot `
                -CacheRoot (Join-Path $overlapRuntimeRoot 'jre') -JreArchive $jreArchive `
                -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar
        }
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $overlapRuntimeRoot 'jre'))) `
        'An overlapping cache path must not create or corrupt a payload destination'

    $linkedCacheTarget = Join-Path $testRoot 'linked-cache-target'
    $linkedCacheRoot = Join-Path $testRoot 'linked-cache-root'
    New-Item -ItemType Directory -Path $linkedCacheTarget | Out-Null
    New-Item -ItemType Junction -Path $linkedCacheRoot -Target $linkedCacheTarget | Out-Null
    $linkedCacheRuntime = Join-Path $testRoot 'runtime-linked-cache'
    Write-FixtureDescriptor -RuntimeRoot $linkedCacheRuntime -JreArchive $jreArchive `
        -MicroemulatorArchive $microemulatorArchive -GameJar $gameJar -JreFiles $jreFiles
    Assert-ThrowsLike -Pattern '*reparse point*' `
        -Because 'A cache root reached through a junction must be rejected before staging' -Action {
            & $provisionerPath -RuntimeRoot $linkedCacheRuntime -CacheRoot $linkedCacheRoot `
                -JreArchive $jreArchive -MicroemulatorArchive $microemulatorArchive `
                -GameJar $gameJar
        }
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $linkedCacheRuntime 'jre'))) `
        'A linked cache rejection must happen before installing payload content'

    Write-Output 'PASS: Runtime provisioning contracts'
}
finally {
    $resolvedTestRoot = [IO.Path]::GetFullPath($testRoot)
    if (-not $resolvedTestRoot.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($resolvedTestRoot) -notlike 'zeus-runtime-provisioning-tests-*') {
        throw "Refusing to clean unexpected test root: $resolvedTestRoot"
    }
    if (Test-Path -LiteralPath $resolvedTestRoot) {
        Remove-Item -LiteralPath $resolvedTestRoot -Recurse -Force
    }
}
