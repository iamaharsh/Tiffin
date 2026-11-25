//! The app's logic without any window, for the Tauri app in `desktop/`.
pub mod cooking;
pub mod credits;
pub mod foods;
pub mod groceries;
pub mod health;
pub mod importer;
#[cfg(feature = "native")]
pub mod inbox;
pub mod ingredients;
pub mod labels;
pub mod model;
pub mod nutrition;
pub mod rules;
pub mod selftest;
#[cfg(feature = "native")]
pub mod shortcut;
pub mod sidebar;
pub mod storage;
pub mod tracker;

/// A folder tests may write in: the system temp folder, or `/tmp` under WASI, where
/// `std::env::temp_dir()` panics (the test runner maps a real folder there).
#[cfg(test)]
pub(crate) fn scratch_dir() -> std::path::PathBuf {
    #[cfg(target_os = "wasi")]
    return std::path::PathBuf::from("/tmp");
    #[cfg(not(target_os = "wasi"))]
    std::env::temp_dir()
}

/// The test process's id, for folder names; 0 under WASI, which has none (one process runs there).
#[cfg(test)]
pub(crate) fn test_pid() -> u32 {
    #[cfg(target_os = "wasi")]
    return 0;
    #[cfg(not(target_os = "wasi"))]
    std::process::id()
}
