//! Observability and structured tracing
//!
//! Provides metrics collection, span builders, and trace log export.

pub mod metrics;
pub mod span;
pub mod trace_log;

pub use metrics::{MetricsCollector, TokenUsage};
pub use span::{hook_span, mcp_span, provider_span, step_span, tool_span};
pub use trace_log::{TraceEvent, TraceLog};
