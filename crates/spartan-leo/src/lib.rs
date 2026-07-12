//! Real §4 Leo agentic core (task #5) -- first real increment. Implements
//! exactly §35.4's own Tier 1 bar for this row: "Plan→approve→execute→
//! verify loop, checkpointing, project-tier memory only." See `agent.rs`'s
//! own doc comment for the full orchestration and what's deliberately out
//! of scope for this pass (sub-agents §4.4, team memory §15, multi-tier
//! memory compaction, threading/UI wiring).

pub mod agent;
pub mod approval;
pub mod checkpoint;
pub mod execute;
pub mod memory;
pub mod persona;
pub mod plan;
pub mod state;
pub mod tool;

pub use agent::{Agent, AgentError};
pub use approval::{may_auto_execute, ApprovalMode, RiskClass};
pub use execute::{append_tool_result, next_action, ExecuteAction, ExecuteError, ExecuteStep};
pub use plan::{generate_plan, ImplementationPlan, PlanError};
pub use state::AgentState;
pub use tool::{Sandbox, SandboxError, ToolCall, ToolResult};
