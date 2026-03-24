// crates/config/src/lib.rs
pub mod registry;
pub mod agent;

pub use registry::{Registry, RegistryEntry};
pub use agent::{AgentDef, AgentPermissions};
