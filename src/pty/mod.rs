mod manager;
mod output;
pub mod proc_identity;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod process_tree;

pub use manager::{PtyHandle, PtyManager, Resume, SessionSpawnConfig};

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod cleanup_tests;
