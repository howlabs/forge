//! Skills framework for reusable prompt + tool combinations
//!
//! Provides discovery, loading, and registry of reusable skill files.

pub mod discovery;
pub mod registry;
pub mod types;

pub use discovery::SkillDiscovery;
pub use registry::{RegisteredSkill, SkillVersion, SkillsRegistry};
pub use types::Skill;
