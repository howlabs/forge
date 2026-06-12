//! Skills framework for reusable prompt + tool combinations
//!
//! Provides discovery and loading of reusable skill files from the skills directory.

pub mod discovery;
pub mod types;

pub use discovery::SkillDiscovery;
pub use types::Skill;
