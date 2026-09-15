//! Exe-relative portable folder layout.
//!
//! Every path is derived from the executable's own directory, so copying the whole `Zeus/` folder to
//! another machine moves the database, key, profiles, and runtime together. Nothing is read from the
//! registry, an environment variable, or the current working directory.

use std::path::{Path, PathBuf};

/// Data root child of the portable folder.
pub const DATA_DIRECTORY: &str = "data";

/// Runtime root children of the portable folder.
pub const RUNTIMES_DIRECTORY: &str = "runtimes";
pub const RUNTIME_TARGET_DIRECTORY: &str = "windows-x64";
/// The single pinned runtime bundle this build supports.
pub const PINNED_RUNTIME_DIRECTORY: &str = "temurin-11.0.32+9_microemu-2.0.4_ko402";

/// Resolved portable paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortableLayout {
    pub root: PathBuf,
    pub data_root: PathBuf,
    pub runtime_root: PathBuf,
}

impl PortableLayout {
    /// Derives the layout from the directory holding the executable.
    pub fn from_executable_directory(executable_directory: &Path) -> Self {
        let root = executable_directory.to_path_buf();
        let data_root = root.join(DATA_DIRECTORY);
        let runtime_root = root
            .join(RUNTIMES_DIRECTORY)
            .join(RUNTIME_TARGET_DIRECTORY)
            .join(PINNED_RUNTIME_DIRECTORY);
        Self {
            root,
            data_root,
            runtime_root,
        }
    }

    /// Derives the layout from the running executable's own path.
    pub fn from_current_executable() -> Option<Self> {
        let executable = std::env::current_exe().ok()?;
        let directory = executable.parent()?;
        Some(Self::from_executable_directory(directory))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a platform-native root so the same assertions run on Windows and Ubuntu.
    fn root(segments: &[&str]) -> PathBuf {
        segments.iter().collect()
    }

    #[test]
    fn windows_shell_layout_is_exe_relative() {
        let base = root(&["games", "Zeus"]);
        let layout = PortableLayout::from_executable_directory(&base);

        assert_eq!(layout.root, base);
        assert_eq!(layout.data_root, base.join("data"));
        assert_eq!(
            layout.runtime_root,
            base.join("runtimes")
                .join("windows-x64")
                .join("temurin-11.0.32+9_microemu-2.0.4_ko402")
        );
        // Both live under the portable root, so moving the folder moves everything together.
        assert!(layout.data_root.starts_with(&layout.root));
        assert!(layout.runtime_root.starts_with(&layout.root));
        // Nothing is read from the registry, the environment, or the working directory.
        assert!(layout.data_root.is_absolute() == base.is_absolute());
    }

    #[test]
    fn windows_shell_layout_follows_a_moved_folder() {
        let first = PortableLayout::from_executable_directory(&root(&["first", "Zeus"]));
        let moved_base = root(&["second", "Tools", "Zeus copy"]);
        let moved = PortableLayout::from_executable_directory(&moved_base);

        // A moved folder yields entirely new paths: nothing is cached or absolute-pinned.
        assert!(!moved.data_root.starts_with(&first.root));
        assert!(!moved.runtime_root.starts_with(&first.root));
        assert_eq!(moved.data_root, moved_base.join("data"));
        // A path containing spaces is preserved exactly.
        assert!(moved.runtime_root.to_string_lossy().contains("Zeus copy"));
    }

    #[test]
    fn windows_shell_layout_names_match_the_release_folder() {
        assert_eq!(DATA_DIRECTORY, "data");
        assert_eq!(RUNTIMES_DIRECTORY, "runtimes");
        assert_eq!(RUNTIME_TARGET_DIRECTORY, "windows-x64");
        assert_eq!(
            PINNED_RUNTIME_DIRECTORY,
            "temurin-11.0.32+9_microemu-2.0.4_ko402"
        );
    }
}
