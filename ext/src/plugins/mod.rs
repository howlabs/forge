//! Plugin system for Forge

pub mod loader;
pub mod registry;
pub mod types;

pub use registry::PluginRegistry;
pub use types::{PluginManifest, PluginMetadata, PluginStatus, PluginVersion};
