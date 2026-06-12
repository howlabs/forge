# Shared Contract for v0.170.0 (Multi-Agent) and v0.180.0 (Long-Horizon)

**Frozen on**: 2025-06-12 (pre-parallel development split)
**Purpose**: Define stable API that both tracks MUST implement before branching

---

## Core Types

### Task - Unit of work for subagents
```rust
pub struct Task {
    pub id: String,              // Unique task identifier (UUID)
    pub prompt: String,           // What the subagent should do
    pub worktree: PathBuf,        // Git worktree path for isolated execution
    pub status: TaskStatus,       // Current state
    pub created_at: std::time::SystemTime,
}

pub enum TaskStatus {
    Pending,      // Created but not started
    Running,      // Subagent is executing
    Verifying,    // Post-execution verification in progress
    Done,         // Completed successfully and passed verification
    Failed,       // Failed during execution or verification
}
```

### VerifyReport - Verification result
```rust
pub struct VerifyReport {
    pub passed: bool,         // Did verification pass?
    pub logs: String,         // Verification output (build + test logs)
    pub duration_ms: u64,     // How long verification took
}
```

### Checkpoint - Crash recovery state
```rust
pub struct Checkpoint {
    pub task_id: String,      // Which task this checkpoint belongs to
    pub step: u32,             // Step number within the task
    pub state: Vec<u8>,        // Serialized state (bincode or JSON)
    pub timestamp: std::time::SystemTime,
}
```

---

## Traits (Both tracks MUST implement)

### Orchestrator - Spawn and manage subagents
```rust
#[async_trait::async_trait]
pub trait Orchestrator: Send + Sync {
    /// Spawn a new subagent to execute a task
    /// Creates isolated git worktree, launches subagent event-loop
    async fn spawn(&mut self, task: Task) -> Result<()>;

    /// Wait for all running subagents to complete
    /// Returns final states of all tasks
    async fn join_all(&mut self) -> Result<Vec<Task>>;

    /// Get current task status without blocking
    fn get_task_status(&self, task_id: &str) -> Option<TaskStatus>;

    /// Cancel a running task
    async fn cancel_task(&mut self, task_id: &str) -> Result<()>;
}
```

### Verifier - Build and test verification
```rust
#[async_trait::async_trait]
pub trait Verifier: Send + Sync {
    /// Run build + tests in a worktree
    /// Returns verification report with pass/fail and logs
    async fn verify(&self, workdir: &Path) -> Result<VerifyReport>;

    /// Quick check if workdir looks buildable
    /// Returns early if obvious issues (missing Cargo.toml, etc.)
    async fn quick_check(&self, workdir: &Path) -> Result<bool>;
}
```

### CheckpointStore - Crash-safe state persistence
```rust
#[async_trait::async_trait]
pub trait CheckpointStore: Send + Sync {
    /// Save checkpoint for a task step
    async fn save(&self, checkpoint: &Checkpoint) -> Result<()>;

    /// Load latest checkpoint for a task
    /// Returns None if no checkpoint exists
    async fn load(&self, task_id: &str) -> Result<Option<Checkpoint>>;

    /// List all checkpointed tasks
    async fn list_tasks(&self) -> Result<Vec<String>>;

    /// Delete all checkpoints for a task
    async fn delete(&self, task_id: &str) -> Result<()>;
}
```

---

## Configuration (forge.toml)

Both tracks MUST respect these config keys:

```toml
[orchestrator]
max_parallel = 4              # Max concurrent subagents (default: CPU count)
subagent_context = "isolated"  # Context isolation strategy (only "isolated" supported v0.170.0)

[checkpoint]
enabled = true               # Enable crash recovery (v0.180.0)
store_path = ".forge/checkpoints"  # Where to store checkpoints
max_checkpoints_per_task = 10  # Rotation policy

[verification]
required = true              # Must pass verify() before marking Task Done
timeout_seconds = 300         # Max time for build+test
```

---

## Version Contract

**v0.170.0 (Orchestrator)**:
- MUST implement Orchestrator trait
- MUST create isolated git worktrees per subagent
- MUST respect max_parallel from forge.toml
- MAY implement stub Verifier (delegates to v0.180.0)

**v0.180.0 (Long-Horizon + Verify)**:
- MUST implement Verifier trait (build + test)
- MUST implement CheckpointStore trait (sqlite/file)
- MUST refuse to mark Task Done unless verify() returns passed=true
- MUST integrate with Orchestrator (post-execution verify gate)

---

## Integration Contract (Post-Merge)

After both tracks merge to main:
1. Orchestrator MUST call Verifier.verify() before setting TaskStatus::Done
2. CheckpointStore MUST be called at each step boundary
3. `forge resume <task_id>` CLI command MUST restore from latest checkpoint
4. Parallel subagents MUST NOT pollute main context (isolated worktrees required)

---

## Non-Goals (Explicitly OUT OF SCOPE)

These are NOT part of v0.170.0/v0.180.0:
- Cross-subagent communication (subagents are isolated)
- Dynamic task graph/dependencies (flat task list only)
- Distributed execution (single machine only)
- Advanced scheduling (FIFO queue only, no priorities)

---

## Testing Contract

Both tracks MUST provide tests for:
- TRACK A (v0.170.0): spawn 2+ subagents in parallel, verify worktree isolation
- TRACK B (v0.180.0): kill mid-run, resume from checkpoint, verify rejection on failed tests
- INTEGRATION: parallel multi-agent run with checkpoint crash survival + verify gate
