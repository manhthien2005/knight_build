use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use crate::error::{CoreError, CoreResult};

#[cfg(windows)]
mod portable_windows;

const ROOT_MARKER_NAME: &str = ".zeus-hso-root";
const ROOT_MARKER: &[u8] = b"ZEUS_HSO_DATA_ROOT\nschema_version=1\n";
const MAX_MARKER_BYTES: u64 = 128;

pub(crate) struct DataRoot {
    path: PathBuf,
}

impl DataRoot {
    pub(crate) fn prepare_at(requested: &Path) -> CoreResult<Self> {
        validate_path_shape(requested)?;
        platform::ensure_local_filesystem(requested)?;
        reject_linked_ancestors(requested)?;

        if requested.exists() {
            let metadata = fs::symlink_metadata(requested)
                .map_err(|error| CoreError::io("inspect data root", error))?;
            if !metadata.is_dir() || platform::is_link_or_reparse(&metadata) {
                return Err(CoreError::InvalidDataRoot {
                    reason: "root is not a plain directory",
                });
            }
            recover_or_validate_existing_root(requested)?;
        } else {
            let parent = requested.parent().ok_or(CoreError::InvalidDataRoot {
                reason: "root has no parent directory",
            })?;
            let parent_metadata = fs::symlink_metadata(parent)
                .map_err(|error| CoreError::io("inspect data-root parent", error))?;
            if !parent_metadata.is_dir() || platform::is_link_or_reparse(&parent_metadata) {
                return Err(CoreError::InvalidDataRoot {
                    reason: "parent is not a plain directory",
                });
            }
            platform::create_private_directory(requested)?;
            write_root_marker(requested)?;
        }

        let canonical = fs::canonicalize(requested)
            .map_err(|error| CoreError::io("canonicalize data root", error))?;
        reject_linked_ancestors(&canonical)?;
        if !platform::is_private_directory(&canonical)? {
            return Err(CoreError::InsecureDataRoot);
        }

        Ok(Self { path: canonical })
    }

    /// Prepares a data root that may have been copied or moved between machines.
    ///
    /// On Windows the bounded portable repair runs before the ordinary preparation path so an
    /// Explorer-style copy carrying inherited ACLs is re-owned and re-protected. Other platforms
    /// share the ordinary path unchanged.
    pub(crate) fn prepare_portable_at(requested: &Path) -> CoreResult<Self> {
        #[cfg(windows)]
        portable_windows::prepare_portable_root(requested)?;
        Self::prepare_at(requested)
    }

    pub(crate) fn prepare_default() -> CoreResult<Self> {
        let default = platform::default_data_root()?;
        Self::prepare_at(&default)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn is_private(&self) -> CoreResult<bool> {
        platform::is_private_directory(&self.path)
    }

    pub(crate) fn state_files_are_private(&self, names: &[&str]) -> CoreResult<bool> {
        for name in std::iter::once(ROOT_MARKER_NAME).chain(names.iter().copied()) {
            let path = self.path.join(name);
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| CoreError::io("inspect private state file", error))?;
            if !metadata.is_file()
                || platform::is_link_or_reparse(&metadata)
                || !platform::is_private_file(&path)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(crate) fn ensure_private_child_directory(&self, name: &str) -> CoreResult<PathBuf> {
        if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
            return Err(CoreError::InvalidDataRoot {
                reason: "invalid internal directory name",
            });
        }
        let child = self.path.join(name);
        if child.exists() {
            let metadata = fs::symlink_metadata(&child)
                .map_err(|error| CoreError::io("inspect internal directory", error))?;
            if !metadata.is_dir()
                || platform::is_link_or_reparse(&metadata)
                || !platform::is_private_child_directory(&child)?
            {
                return Err(CoreError::InvalidDataRoot {
                    reason: "internal directory is linked, insecure or not a directory",
                });
            }
        } else {
            platform::create_private_child_directory(&child)?;
        }
        Ok(child)
    }

    pub(crate) fn create_profile_directory(&self, profile_id: &str) -> CoreResult<PathBuf> {
        let profiles = self.ensure_private_child_directory("profiles")?;
        let child = profiles.join(profile_id);
        platform::create_private_child_directory(&child)?;
        if !platform::is_private_child_directory(&child)? {
            let _ = fs::remove_dir(&child);
            let _ = platform::sync_directory(&profiles);
            return Err(CoreError::InsecureDataRoot);
        }
        Ok(child)
    }

    pub(crate) fn profile_directory(&self, profile_id: &str) -> PathBuf {
        self.path.join("profiles").join(profile_id)
    }

    pub(crate) fn verified_profile_directory(&self, profile_id: &str) -> CoreResult<PathBuf> {
        let path = self.profile_directory(profile_id);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| CoreError::io("inspect profile directory", error))?;
        if !metadata.is_dir()
            || platform::is_link_or_reparse(&metadata)
            || !platform::is_private_child_directory(&path)?
        {
            return Err(CoreError::InsecureDataRoot);
        }
        Ok(path)
    }

    pub(crate) fn ensure_private_profile_child_directory(
        &self,
        profile_id: &str,
        name: &str,
    ) -> CoreResult<PathBuf> {
        if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
            return Err(CoreError::InvalidDataRoot {
                reason: "invalid profile child directory name",
            });
        }
        let profile = self.verified_profile_directory(profile_id)?;
        reject_linked_ancestors(&profile)?;
        let child = profile.join(name);
        if child.exists() {
            let metadata = fs::symlink_metadata(&child)
                .map_err(|error| CoreError::io("inspect profile child directory", error))?;
            if !metadata.is_dir()
                || platform::is_link_or_reparse(&metadata)
                || !platform::is_private_child_directory(&child)?
            {
                return Err(CoreError::InsecureDataRoot);
            }
        } else {
            platform::create_private_child_directory(&child)?;
        }
        let canonical = fs::canonicalize(&child)
            .map_err(|error| CoreError::io("canonicalize profile child directory", error))?;
        reject_linked_ancestors(&canonical)?;
        if canonical.parent() != Some(profile.as_path())
            || !platform::is_private_child_directory(&canonical)?
        {
            return Err(CoreError::InsecureDataRoot);
        }
        Ok(canonical)
    }

    pub(crate) fn remove_empty_profile_directory(&self, profile_id: &str) -> CoreResult<()> {
        let path = self.profile_directory(profile_id);
        fs::remove_dir(&path)
            .map_err(|error| CoreError::io("remove uncommitted profile directory", error))?;
        if let Some(parent) = path.parent() {
            platform::sync_directory(parent)?;
        }
        Ok(())
    }
}

pub(crate) fn open_private_file(path: &Path) -> CoreResult<File> {
    reject_non_plain_existing_file(path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    platform::set_private_create_mode(&mut options);
    let file = options
        .open(path)
        .map_err(|error| CoreError::io("open private state file", error))?;
    platform::enforce_private_file_mode(path)?;
    Ok(file)
}

pub(crate) fn create_private_truncated_file(path: &Path) -> CoreResult<File> {
    reject_non_plain_existing_file(path)?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    platform::set_private_create_mode(&mut options);
    let file = options
        .open(path)
        .map_err(|error| CoreError::io("create private state file", error))?;
    platform::enforce_private_file_mode(path)?;
    Ok(file)
}

pub(crate) fn harden_existing_private_file(path: &Path) -> CoreResult<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| CoreError::io("inspect existing private state file", error))?;
    if !metadata.is_file() || platform::is_link_or_reparse(&metadata) {
        return Err(CoreError::InvalidDataRoot {
            reason: "state path is linked or not a regular file",
        });
    }
    if !platform::is_private_file_compatible(path)? {
        return Err(CoreError::InsecureDataRoot);
    }
    platform::enforce_private_file_mode(path)
}

pub fn atomic_replace(source: &Path, destination: &Path) -> CoreResult<()> {
    platform::atomic_replace(source, destination)
}

pub(crate) fn canonical_secure_runtime_root(path: &Path) -> CoreResult<PathBuf> {
    validate_path_shape(path)?;
    platform::ensure_local_filesystem(path)?;
    reject_linked_ancestors(path)?;
    let metadata =
        fs::symlink_metadata(path).map_err(|error| CoreError::io("inspect runtime root", error))?;
    if !metadata.is_dir() || platform::is_link_or_reparse(&metadata) {
        return Err(CoreError::InvalidDataRoot {
            reason: "runtime root is not a plain directory",
        });
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| CoreError::io("canonicalize runtime root", error))?;
    reject_linked_ancestors(&canonical)?;
    if !platform::is_secure_runtime_directory(&canonical)? {
        return Err(CoreError::InsecureDataRoot);
    }
    Ok(canonical)
}

pub(crate) fn reject_linked_path(path: &Path) -> CoreResult<()> {
    reject_linked_ancestors(path)
}

pub(crate) fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    platform::is_link_or_reparse(metadata)
}

pub(crate) fn secure_runtime_path(root: &Path, path: &Path) -> CoreResult<bool> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| CoreError::InvalidDataRoot {
            reason: "runtime entry escaped its root",
        })?;
    let root_metadata = fs::symlink_metadata(root)
        .map_err(|error| CoreError::io("inspect runtime root permissions", error))?;
    if platform::is_link_or_reparse(&root_metadata)
        || !platform::is_secure_runtime_entry(root, &root_metadata)?
    {
        return Ok(false);
    }
    let mut current = root.to_owned();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Ok(false);
        }
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| CoreError::io("inspect runtime entry permissions", error))?;
        if platform::is_link_or_reparse(&metadata)
            || !platform::is_secure_runtime_entry(&current, &metadata)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn runtime_entry_is_secure(path: &Path, metadata: &fs::Metadata) -> CoreResult<bool> {
    platform::is_secure_runtime_entry(path, metadata)
}

fn validate_path_shape(path: &Path) -> CoreResult<()> {
    if !path.is_absolute() {
        return Err(CoreError::InvalidDataRoot {
            reason: "root must be absolute",
        });
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(CoreError::InvalidDataRoot {
            reason: "root cannot contain relative traversal",
        });
    }
    platform::validate_absolute_prefix(path)
}

fn reject_linked_ancestors(path: &Path) -> CoreResult<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if !current.exists() {
            continue;
        }
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| CoreError::io("inspect path component", error))?;
        if platform::is_link_or_reparse(&metadata) {
            return Err(CoreError::InvalidDataRoot {
                reason: "root traverses a link or reparse point",
            });
        }
    }
    Ok(())
}

fn recover_or_validate_existing_root(root: &Path) -> CoreResult<()> {
    let marker = root.join(ROOT_MARKER_NAME);
    if marker.exists() {
        if !platform::is_private_directory(root)? {
            return Err(CoreError::InsecureDataRoot);
        }
        validate_root_marker(&marker)?;
        harden_existing_private_file(&marker)?;
        return Ok(());
    }

    let mut entries = fs::read_dir(root)
        .map_err(|error| CoreError::io("enumerate unmanaged data root", error))?;
    if entries
        .next()
        .transpose()
        .map_err(|error| CoreError::io("enumerate unmanaged data-root entry", error))?
        .is_some()
    {
        return Err(CoreError::UnmanagedDataRoot);
    }
    if !platform::is_private_directory(root)? {
        return Err(CoreError::InsecureDataRoot);
    }
    write_root_marker(root)
}

fn reject_non_plain_existing_file(path: &Path) -> CoreResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(CoreError::io("inspect private state path", error)),
    };
    if !metadata.is_file() || platform::is_link_or_reparse(&metadata) {
        return Err(CoreError::InvalidDataRoot {
            reason: "state path is linked or not a regular file",
        });
    }
    if !platform::is_private_file_compatible(path)? {
        return Err(CoreError::InsecureDataRoot);
    }
    Ok(())
}

fn write_root_marker(root: &Path) -> CoreResult<()> {
    let marker = root.join(ROOT_MARKER_NAME);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    platform::set_private_create_mode(&mut options);
    let mut file = options
        .open(&marker)
        .map_err(|error| CoreError::io("create data-root marker", error))?;
    file.write_all(ROOT_MARKER)
        .and_then(|()| file.sync_all())
        .map_err(|error| CoreError::io("persist data-root marker", error))?;
    platform::enforce_private_file_mode(&marker)?;
    platform::sync_directory(root)
}

fn validate_root_marker(marker: &Path) -> CoreResult<()> {
    let metadata = fs::symlink_metadata(marker)
        .map_err(|error| CoreError::io("inspect data-root marker", error))?;
    if !metadata.is_file()
        || platform::is_link_or_reparse(&metadata)
        || metadata.len() > MAX_MARKER_BYTES
    {
        return Err(CoreError::UnmanagedDataRoot);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(marker)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| CoreError::io("read data-root marker", error))?;
    if bytes != ROOT_MARKER {
        return Err(CoreError::UnmanagedDataRoot);
    }
    Ok(())
}

#[cfg(windows)]
mod platform {
    use std::ffi::{OsString, c_void};
    use std::fs::{Metadata, OpenOptions};
    use std::io;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::MetadataExt;
    use std::path::{Component, Path, PathBuf, Prefix};
    use std::ptr::{addr_of, null_mut};
    use std::slice;

    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation, GetFileSecurityW,
        GetSecurityDescriptorControl, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner,
        GetTokenInformation, INHERITED_ACE, IsWellKnownSid, OWNER_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
        SECURITY_ATTRIBUTES, SetFileSecurityW, TOKEN_QUERY, TOKEN_USER, TokenUser,
        WinLocalSystemSid,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateDirectoryW, GetDriveTypeW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        MoveFileExW,
    };
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows_sys::Win32::UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath};

    use crate::error::{CoreError, CoreResult};

    const FILE_ATTRIBUTE_REPARSE_POINT_VALUE: u32 = 0x0000_0400;
    const DRIVE_FIXED_VALUE: u32 = 3;
    const ACCESS_ALLOWED_ACE_TYPE_VALUE: u8 = 0;

    pub(super) fn default_data_root() -> CoreResult<PathBuf> {
        let mut raw_path = null_mut();
        // SAFETY: `raw_path` is a valid out pointer and a null token requests the current user.
        let result =
            unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, 0, null_mut(), &mut raw_path) };
        if result < 0 || raw_path.is_null() {
            return Err(CoreError::io(
                "resolve per-user LocalAppData",
                io::Error::from_raw_os_error(result),
            ));
        }
        let length = wide_len(raw_path);
        // SAFETY: SHGetKnownFolderPath returned a NUL-terminated buffer of at least `length`.
        let path = unsafe { OsString::from_wide(slice::from_raw_parts(raw_path, length)) };
        // SAFETY: the buffer is allocated by COM and must be released with CoTaskMemFree.
        unsafe { CoTaskMemFree(raw_path.cast()) };
        Ok(PathBuf::from(path).join("Zeus_HSO"))
    }

    pub(super) fn validate_absolute_prefix(path: &Path) -> CoreResult<()> {
        match path.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(_) | Prefix::VerbatimDisk(_) => Ok(()),
                _ => Err(CoreError::InvalidDataRoot {
                    reason: "UNC and device paths are unsupported",
                }),
            },
            _ => Err(CoreError::InvalidDataRoot {
                reason: "root must use a drive-letter path",
            }),
        }
    }

    pub(super) fn ensure_local_filesystem(path: &Path) -> CoreResult<()> {
        let letter = match path.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
                _ => {
                    return Err(CoreError::InvalidDataRoot {
                        reason: "network or device roots are unsupported",
                    });
                }
            },
            _ => {
                return Err(CoreError::InvalidDataRoot {
                    reason: "root has no drive prefix",
                });
            }
        };
        let drive = [letter as u16, b':' as u16, b'\\' as u16, 0];
        // SAFETY: `drive` is a valid NUL-terminated drive-root string.
        let drive_type = unsafe { GetDriveTypeW(drive.as_ptr()) };
        if drive_type != DRIVE_FIXED_VALUE {
            return Err(CoreError::InvalidDataRoot {
                reason: "data root must be on a fixed local drive",
            });
        }
        Ok(())
    }

    pub(super) fn is_link_or_reparse(metadata: &Metadata) -> bool {
        metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT_VALUE != 0
    }

    pub(super) fn create_private_directory(path: &Path) -> CoreResult<()> {
        let descriptor = LocalSecurityDescriptor::for_current_user()?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        let path = to_wide(path)?;
        // SAFETY: both pointers reference initialized, live structures for the duration of the call.
        if unsafe { CreateDirectoryW(path.as_ptr(), &attributes) } == 0 {
            return Err(CoreError::io(
                "create private data root",
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    pub(super) fn create_private_child_directory(path: &Path) -> CoreResult<()> {
        std::fs::create_dir(path)
            .map_err(|error| CoreError::io("create private internal directory", error))
    }

    pub(super) fn is_private_directory(path: &Path) -> CoreResult<bool> {
        verify_private_acl(path)
    }

    pub(super) fn is_private_child_directory(path: &Path) -> CoreResult<bool> {
        verify_restricted_acl(path, true)
    }

    pub(super) fn is_private_file(path: &Path) -> CoreResult<bool> {
        verify_private_acl(path)
    }

    pub(super) fn is_private_file_compatible(path: &Path) -> CoreResult<bool> {
        verify_restricted_acl(path, true)
    }

    pub(super) fn is_secure_runtime_directory(path: &Path) -> CoreResult<bool> {
        verify_private_acl(path)
    }

    pub(super) fn is_secure_runtime_entry(path: &Path, _metadata: &Metadata) -> CoreResult<bool> {
        verify_restricted_acl(path, true)
    }

    pub(super) fn set_private_create_mode(_options: &mut OpenOptions) {}

    pub(super) fn enforce_private_file_mode(path: &Path) -> CoreResult<()> {
        let descriptor = LocalSecurityDescriptor::for_current_user()?;
        let wide = to_wide(path)?;
        // SAFETY: the path and security descriptor are valid, live buffers for this call.
        if unsafe {
            SetFileSecurityW(
                wide.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor.0,
            )
        } == 0
        {
            return Err(CoreError::io(
                "set private state-file ACL",
                io::Error::last_os_error(),
            ));
        }
        if !verify_private_acl(path)? {
            return Err(CoreError::InsecureDataRoot);
        }
        Ok(())
    }

    pub(super) fn owner_is_current_user(path: &Path) -> CoreResult<bool> {
        let wide = to_wide(path)?;
        let mut needed = 0u32;
        // SAFETY: the first call intentionally supplies an empty buffer to obtain the size.
        unsafe {
            GetFileSecurityW(
                wide.as_ptr(),
                OWNER_SECURITY_INFORMATION,
                null_mut(),
                0,
                &mut needed,
            );
        }
        if needed == 0 {
            return Err(CoreError::io(
                "size portable entry owner",
                io::Error::last_os_error(),
            ));
        }
        let mut words = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
        let descriptor = words.as_mut_ptr().cast();
        // SAFETY: the aligned buffer holds at least `needed` bytes and every pointer is valid.
        if unsafe {
            GetFileSecurityW(
                wide.as_ptr(),
                OWNER_SECURITY_INFORMATION,
                descriptor,
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(CoreError::io(
                "read portable entry owner",
                io::Error::last_os_error(),
            ));
        }
        let current = CurrentUserSid::read()?;
        let mut owner = null_mut();
        let mut defaulted = 0;
        // SAFETY: `descriptor` holds a security descriptor returned by GetFileSecurityW.
        if unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted) } == 0
            || owner.is_null()
        {
            return Ok(false);
        }
        // SAFETY: both SIDs are valid for the duration of this comparison.
        Ok(unsafe { EqualSid(owner, current.as_psid()) } != 0)
    }

    /// Assigns the current user as owner, reporting `false` when the token lacks the privilege.
    pub(super) fn assign_current_user_owner(path: &Path) -> CoreResult<bool> {
        let descriptor = LocalSecurityDescriptor::for_current_user_owner()?;
        let wide = to_wide(path)?;
        // SAFETY: the path and the owner-only descriptor are valid, live buffers for this call.
        if unsafe { SetFileSecurityW(wide.as_ptr(), OWNER_SECURITY_INFORMATION, descriptor.0) } == 0
        {
            return Ok(false);
        }
        Ok(true)
    }

    pub(super) fn atomic_replace(source: &Path, destination: &Path) -> CoreResult<()> {
        let source = to_wide(source)?;
        let destination = to_wide(destination)?;
        // SAFETY: both paths are valid NUL-terminated strings and flags request an atomic replace.
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(CoreError::io(
                "replace last-known-good database backup",
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    pub(super) fn sync_directory(_path: &Path) -> CoreResult<()> {
        Ok(())
    }

    struct TokenHandle(*mut c_void);

    impl Drop for TokenHandle {
        fn drop(&mut self) {
            // SAFETY: the handle was returned by OpenProcessToken and is owned by this wrapper.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    struct CurrentUserSid {
        words: Vec<usize>,
    }

    impl CurrentUserSid {
        fn read() -> CoreResult<Self> {
            let mut token = null_mut();
            // SAFETY: output pointer is valid and GetCurrentProcess returns a process pseudo-handle.
            if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
                return Err(CoreError::io(
                    "open current process token",
                    io::Error::last_os_error(),
                ));
            }
            let token = TokenHandle(token);
            let mut needed = 0u32;
            // SAFETY: the first call intentionally supplies no buffer to obtain the required size.
            unsafe {
                GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut needed);
            }
            if needed == 0 {
                return Err(CoreError::io(
                    "size current user SID",
                    io::Error::last_os_error(),
                ));
            }
            let word_count = (needed as usize).div_ceil(size_of::<usize>());
            let mut words = vec![0usize; word_count];
            // SAFETY: the aligned allocation is at least `needed` bytes and the out length is valid.
            if unsafe {
                GetTokenInformation(
                    token.0,
                    TokenUser,
                    words.as_mut_ptr().cast(),
                    needed,
                    &mut needed,
                )
            } == 0
            {
                return Err(CoreError::io(
                    "read current user SID",
                    io::Error::last_os_error(),
                ));
            }
            Ok(Self { words })
        }

        fn as_psid(&self) -> PSID {
            // SAFETY: GetTokenInformation initialized the allocation as TOKEN_USER.
            unsafe { (*(self.words.as_ptr().cast::<TOKEN_USER>())).User.Sid }
        }

        fn as_string(&self) -> CoreResult<String> {
            let mut raw = null_mut();
            // SAFETY: `as_psid` points to a valid SID for the lifetime of `self`.
            if unsafe { ConvertSidToStringSidW(self.as_psid(), &mut raw) } == 0 {
                return Err(CoreError::io(
                    "format current user SID",
                    io::Error::last_os_error(),
                ));
            }
            let length = wide_len(raw);
            // SAFETY: ConvertSidToStringSidW returned a NUL-terminated LocalAlloc buffer.
            let value =
                unsafe { String::from_utf16(slice::from_raw_parts(raw, length)) }.map_err(|_| {
                    CoreError::InvalidDataRoot {
                        reason: "current user SID is not valid UTF-16",
                    }
                });
            // SAFETY: the SID string was allocated with LocalAlloc.
            unsafe {
                LocalFree(raw.cast());
            }
            value
        }
    }

    struct LocalSecurityDescriptor(PSECURITY_DESCRIPTOR);

    impl LocalSecurityDescriptor {
        fn for_current_user() -> CoreResult<Self> {
            let sid = CurrentUserSid::read()?.as_string()?;
            Self::from_sddl(&format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)"))
        }

        fn for_current_user_owner() -> CoreResult<Self> {
            let sid = CurrentUserSid::read()?.as_string()?;
            Self::from_sddl(&format!("O:{sid}"))
        }

        fn from_sddl(sddl: &str) -> CoreResult<Self> {
            let wide = to_wide_str(sddl)?;
            let mut descriptor = null_mut();
            // SAFETY: the SDDL buffer and output pointer are valid for the call.
            if unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide.as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    null_mut(),
                )
            } == 0
            {
                return Err(CoreError::io(
                    "build private security descriptor",
                    io::Error::last_os_error(),
                ));
            }
            Ok(Self(descriptor))
        }
    }

    impl Drop for LocalSecurityDescriptor {
        fn drop(&mut self) {
            // SAFETY: the descriptor was allocated by ConvertStringSecurityDescriptor... via LocalAlloc.
            unsafe {
                LocalFree(self.0);
            }
        }
    }

    fn verify_private_acl(path: &Path) -> CoreResult<bool> {
        verify_restricted_acl(path, false)
    }

    fn verify_restricted_acl(path: &Path, allow_inherited: bool) -> CoreResult<bool> {
        let path = to_wide(path)?;
        let requested = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut needed = 0u32;
        // SAFETY: first call intentionally supplies an empty buffer to obtain the size.
        unsafe {
            GetFileSecurityW(path.as_ptr(), requested, null_mut(), 0, &mut needed);
        }
        if needed == 0 {
            return Err(CoreError::io(
                "size data-root security descriptor",
                io::Error::last_os_error(),
            ));
        }
        let word_count = (needed as usize).div_ceil(size_of::<usize>());
        let mut descriptor_words = vec![0usize; word_count];
        let descriptor = descriptor_words.as_mut_ptr().cast();
        // SAFETY: the aligned buffer is at least `needed` bytes and all out pointers are valid.
        if unsafe { GetFileSecurityW(path.as_ptr(), requested, descriptor, needed, &mut needed) }
            == 0
        {
            return Err(CoreError::io(
                "read data-root security descriptor",
                io::Error::last_os_error(),
            ));
        }

        let current = CurrentUserSid::read()?;
        if !allow_inherited {
            let mut control = 0u16;
            let mut revision = 0u32;
            // SAFETY: the descriptor and out pointers are valid initialized storage.
            if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0
                || control & SE_DACL_PROTECTED == 0
            {
                return Ok(false);
            }
        }
        let mut owner = null_mut();
        let mut owner_defaulted = 0;
        // SAFETY: `descriptor` contains a security descriptor returned by GetFileSecurityW.
        if unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut owner_defaulted) } == 0
            || owner.is_null()
            || unsafe { EqualSid(owner, current.as_psid()) } == 0
        {
            return Ok(false);
        }

        let mut dacl_present = 0;
        let mut dacl: *mut ACL = null_mut();
        let mut dacl_defaulted = 0;
        // SAFETY: all pointers refer to initialized storage and a valid security descriptor.
        if unsafe {
            GetSecurityDescriptorDacl(
                descriptor,
                &mut dacl_present,
                &mut dacl,
                &mut dacl_defaulted,
            )
        } == 0
            || dacl_present == 0
            || dacl.is_null()
        {
            return Ok(false);
        }
        let mut info: ACL_SIZE_INFORMATION = unsafe { zeroed() };
        // SAFETY: `dacl` is owned by the descriptor buffer and `info` has the requested layout.
        if unsafe {
            GetAclInformation(
                dacl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } == 0
            || info.AceCount != 2
        {
            return Ok(false);
        }

        let mut current_count = 0u32;
        let mut system_count = 0u32;
        for index in 0..info.AceCount {
            let mut raw_ace = null_mut();
            // SAFETY: index is bounded by the ACE count returned for this ACL.
            if unsafe { GetAce(dacl, index, &mut raw_ace) } == 0 || raw_ace.is_null() {
                return Ok(false);
            }
            // SAFETY: GetAce returned a pointer to an ACE whose header is always present.
            let header = unsafe { &*raw_ace.cast::<ACE_HEADER>() };
            if header.AceType != ACCESS_ALLOWED_ACE_TYPE_VALUE
                || (!allow_inherited && u32::from(header.AceFlags) & INHERITED_ACE != 0)
                || usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>()
            {
                return Ok(false);
            }
            // SAFETY: the type and size checks above establish ACCESS_ALLOWED_ACE layout.
            let ace = unsafe { &*raw_ace.cast::<ACCESS_ALLOWED_ACE>() };
            let sid = addr_of!(ace.SidStart).cast_mut().cast::<c_void>();
            // SAFETY: SidStart is the first word of a variable-length SID within this ACE.
            if unsafe { EqualSid(sid, current.as_psid()) } != 0 {
                current_count += 1;
            } else if unsafe { IsWellKnownSid(sid, WinLocalSystemSid) } != 0 {
                system_count += 1;
            } else {
                return Ok(false);
            }
        }
        Ok(current_count == 1 && system_count == 1)
    }

    fn to_wide(path: &Path) -> CoreResult<Vec<u16>> {
        let encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
        if encoded.contains(&0) {
            return Err(CoreError::InvalidDataRoot {
                reason: "path contains a NUL code unit",
            });
        }
        Ok(encoded.into_iter().chain([0]).collect())
    }

    fn to_wide_str(value: &str) -> CoreResult<Vec<u16>> {
        if value.encode_utf16().any(|unit| unit == 0) {
            return Err(CoreError::InvalidDataRoot {
                reason: "security descriptor contains a NUL code unit",
            });
        }
        Ok(value.encode_utf16().chain([0]).collect())
    }

    fn wide_len(value: *const u16) -> usize {
        let mut length = 0usize;
        // SAFETY: callers only pass pointers returned by Win32 APIs as NUL-terminated strings.
        unsafe {
            while *value.add(length) != 0 {
                length += 1;
            }
        }
        length
    }
}

#[cfg(unix)]
mod platform {
    use std::ffi::CString;
    use std::fs::{DirBuilder, Metadata, OpenOptions, Permissions};
    use std::mem::zeroed;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    use crate::error::{CoreError, CoreResult};

    pub(super) fn default_data_root() -> CoreResult<PathBuf> {
        if let Some(value) = std::env::var_os("XDG_DATA_HOME") {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                return Ok(path.join("zeus-hso"));
            }
        }
        let home = std::env::var_os("HOME").ok_or(CoreError::InvalidDataRoot {
            reason: "HOME is unavailable and XDG_DATA_HOME is not absolute",
        })?;
        let home = PathBuf::from(home);
        if !home.is_absolute() {
            return Err(CoreError::InvalidDataRoot {
                reason: "HOME is not absolute",
            });
        }
        Ok(home.join(".local/share/zeus-hso"))
    }

    pub(super) fn validate_absolute_prefix(_path: &Path) -> CoreResult<()> {
        Ok(())
    }

    pub(super) fn ensure_local_filesystem(path: &Path) -> CoreResult<()> {
        #[cfg(target_os = "linux")]
        {
            let existing = path
                .ancestors()
                .find(|candidate| candidate.exists())
                .ok_or(CoreError::InvalidDataRoot {
                    reason: "root has no existing ancestor",
                })?;
            let encoded = CString::new(existing.as_os_str().as_bytes()).map_err(|_| {
                CoreError::InvalidDataRoot {
                    reason: "path contains a NUL byte",
                }
            })?;
            let mut info: libc::statfs = unsafe { zeroed() };
            // SAFETY: the path is NUL-terminated and `info` points to writable storage.
            if unsafe { libc::statfs(encoded.as_ptr(), &mut info) } != 0 {
                return Err(CoreError::io(
                    "inspect data-root filesystem",
                    std::io::Error::last_os_error(),
                ));
            }
            let fs_type = info.f_type as i64;
            let allowed = [
                0x0000_ef53_i64, // ext2/3/4
                0x5846_5342_i64, // XFS
                0x9123_683e_i64, // Btrfs
                0x0102_1994_i64, // tmpfs
                0x2fc1_2fc1_i64, // ZFS
                0x794c_7630_i64, // overlayfs
            ];
            if !allowed.contains(&fs_type) {
                return Err(CoreError::InvalidDataRoot {
                    reason: "filesystem is not in the supported local-filesystem allowlist",
                });
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = path;
            Err(CoreError::InvalidDataRoot {
                reason: "this Unix platform has no validated local-filesystem adapter",
            })
        }
    }

    pub(super) fn is_link_or_reparse(metadata: &Metadata) -> bool {
        metadata.file_type().is_symlink()
    }

    pub(super) fn create_private_directory(path: &Path) -> CoreResult<()> {
        let mut builder = DirBuilder::new();
        builder.mode(0o700);
        builder
            .create(path)
            .map_err(|error| CoreError::io("create private data root", error))?;
        let durability = sync_directory(path)
            .and_then(|()| path.parent().map(sync_directory).unwrap_or_else(|| Ok(())));
        if let Err(error) = durability {
            let _ = std::fs::remove_dir(path);
            if let Some(parent) = path.parent() {
                let _ = sync_directory(parent);
            }
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn create_private_child_directory(path: &Path) -> CoreResult<()> {
        create_private_directory(path)
    }

    pub(super) fn is_private_directory(path: &Path) -> CoreResult<bool> {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| CoreError::io("inspect data-root permissions", error))?;
        Ok(metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0)
    }

    pub(super) fn is_private_child_directory(path: &Path) -> CoreResult<bool> {
        is_private_directory(path)
    }

    pub(super) fn is_private_file(path: &Path) -> CoreResult<bool> {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| CoreError::io("inspect state-file permissions", error))?;
        Ok(metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0)
    }

    pub(super) fn is_private_file_compatible(path: &Path) -> CoreResult<bool> {
        is_private_file(path)
    }

    pub(super) fn is_secure_runtime_directory(path: &Path) -> CoreResult<bool> {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| CoreError::io("inspect runtime-root permissions", error))?;
        Ok(metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o022 == 0)
    }

    pub(super) fn is_secure_runtime_entry(_path: &Path, metadata: &Metadata) -> CoreResult<bool> {
        Ok((metadata.is_dir() || metadata.is_file())
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o022 == 0)
    }

    pub(super) fn set_private_create_mode(options: &mut OpenOptions) {
        options.mode(0o600);
    }

    pub(super) fn enforce_private_file_mode(path: &Path) -> CoreResult<()> {
        std::fs::set_permissions(path, Permissions::from_mode(0o600))
            .map_err(|error| CoreError::io("set private state-file mode", error))?;
        if !is_private_file(path)? {
            return Err(CoreError::InsecureDataRoot);
        }
        Ok(())
    }

    pub(super) fn atomic_replace(source: &Path, destination: &Path) -> CoreResult<()> {
        std::fs::rename(source, destination)
            .map_err(|error| CoreError::io("replace last-known-good database backup", error))?;
        let parent = destination.parent().ok_or(CoreError::InvalidDataRoot {
            reason: "backup destination has no parent directory",
        })?;
        sync_directory(parent)
    }

    pub(super) fn sync_directory(path: &Path) -> CoreResult<()> {
        std::fs::File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| CoreError::io("sync filesystem directory", error))
    }
}
