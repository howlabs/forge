//! Hooks system for lifecycle events
//!
//! Provides framework for executing user-defined scripts at key points
//! in the task lifecycle (creation, completion, edits, verification).

pub mod registry;
pub mod types;

pub use registry::HookRegistry;
pub use types::{HookEvent, PostVerifyEvent, PreEditEvent, TaskCompletedEvent, TaskCreatedEvent};
