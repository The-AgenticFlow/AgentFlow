// crates/pocketflow-core/src/action.rs

/// A typed label that drives routing in the Flow.
/// All routing constants live here so main.rs wiring is self-documenting.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Action(pub String);

impl Action {
    // ── NEXUS actions ──────────────────────────────────────────────────
    pub const TICKETS_READY: &'static str = "tickets_ready";
    pub const AWAITING_HUMAN: &'static str = "awaiting_human";
    pub const NO_TICKETS: &'static str = "no_tickets";
    pub const REASSIGN_TO_FORGE: &'static str = "reassign_to_forge";
    pub const SPRINT_SUSPENDED: &'static str = "sprint_suspended";

    // ── FORGE actions ──────────────────────────────────────────────────
    pub const PR_OPENED: &'static str = "pr_opened";
    pub const BLOCKED: &'static str = "blocked";
    pub const FUEL_EXHAUSTED: &'static str = "fuel_exhausted";
    pub const TASK_FAILED: &'static str = "task_failed"; // distinct from fuel exhaustion

    // ── SENTINEL actions ───────────────────────────────────────────────
    pub const APPROVED: &'static str = "approved";
    pub const CHANGES_REQUESTED: &'static str = "changes_requested";

    // ── VESSEL actions ─────────────────────────────────────────────────
    pub const DEPLOYED: &'static str = "deployed";
    pub const DEPLOY_FAILED: &'static str = "deploy_failed";
    pub const CI_FIX_NEEDED: &'static str = "ci_fix_needed";

    // ── LORE actions ───────────────────────────────────────────────────
    pub const DOCUMENTED: &'static str = "documented";

    /// Construct from any string (for pattern-matching in Flow routing).
    pub fn new(s: impl Into<String>) -> Self {
        Action(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for Action {
    fn from(s: &str) -> Self {
        Action(s.to_string())
    }
}
