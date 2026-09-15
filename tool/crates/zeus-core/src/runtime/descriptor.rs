use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeDescriptor {
    pub schema_version: u32,
    pub runtime_id: String,
    pub created_at_utc: String,
    pub platform: PlatformDescriptor,
    pub java: JavaDescriptor,
    pub microemulator: MicroEmulatorDescriptor,
    pub game: GameDescriptor,
    pub launch_defaults: LaunchDefaultsDescriptor,
    pub validation: ValidationDescriptor,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlatformDescriptor {
    pub os: String,
    pub architecture: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JavaDescriptor {
    pub vendor: String,
    pub distribution: String,
    pub jvm: String,
    pub version: String,
    pub image_type: String,
    pub archive_name: String,
    pub archive_size: u64,
    pub archive_sha256: String,
    pub source: String,
    pub tree_manifest: String,
    pub tree_file_count: usize,
    pub tree_manifest_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MicroEmulatorDescriptor {
    pub version: String,
    pub archive_name: String,
    pub archive_size: u64,
    pub archive_sha256: String,
    pub source: String,
    pub jar: String,
    pub jar_size: u64,
    pub jar_sha256: String,
    pub optional_jars: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GameDescriptor {
    pub name: String,
    pub bundle: String,
    pub midlet_version: String,
    pub profile: String,
    pub configuration: String,
    pub jar: String,
    pub jar_size: u64,
    pub jar_sha256: String,
    pub source_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaunchDefaultsDescriptor {
    pub mode: String,
    pub main_class: String,
    pub midlet_class: String,
    pub screen_width: u32,
    pub screen_height: u32,
    pub heap_initial_mib: u32,
    pub heap_max_mib: u32,
    pub gc: String,
    pub use_perf_data: bool,
    pub rms: String,
    pub quiet: bool,
    pub quit_on_midlet_destroy: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ValidationDescriptor {
    pub status: String,
    pub evidence: String,
    pub passed: Vec<String>,
    pub pending: Vec<String>,
}
