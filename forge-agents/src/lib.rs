//! Multi-agent orchestration (v0.170.0) and Long-horizon checkpointing (v0.180.0)
//!
//! This crate provides:
//! - Shared types: Task, TaskStatus, VerifyReport, Checkpoint
//! - Core traits: Orchestrator, Verifier, CheckpointStore
//! - Frozen contract for parallel development tracks

pub mod types;
pub mod traits;
pub mod orchestrator;

// Re-export core types and traits for convenience
pub use types::{Task, TaskStatus, VerifyReport, Checkpoint};
pub use traits::{Orchestrator, Verifier, CheckpointStore};

#[cfg(test)]
mod tests {
    #[test]
    fn test_shared_contract() {
        // Verify shared contract is properly exported
        use crate::{Task, TaskStatus, VerifyReport, Checkpoint};
        use crate::{Orchestrator, Verifier, CheckpointStore};

        // Test that types can be created
        let task = Task::new("Test", std::path::PathBuf::from("/tmp"));
        assert_eq!(task.status, TaskStatus::Pending);

        let report = VerifyReport::success("OK", 100);
        assert!(report.passed);

        let checkpoint = Checkpoint::new("task-1", 1, vec![1, 2, 3]);
        assert_eq!(checkpoint.step, 1);
    }
}

