use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

// --- Project ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub key: String,
    #[serde(default)]
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub next_id: u64,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct CreateProject {
    pub name: String,
    pub key: String,
    #[serde(default)]
    pub description: String,
    /// Scope this project to a universe. Set server-side from UNIVERSE_KEY env
    /// when not provided by the client.
    #[serde(default)]
    pub universe_key: Option<String>,
}

// --- Task ---

/// API response representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub key: String,
    pub project_key: String,
    pub title: String,
    pub status: TaskStatus,
    pub priority: Priority,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<u64>,
    pub labels: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub description: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTask {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_status")]
    pub status: TaskStatus,
    #[serde(default = "default_priority")]
    pub priority: Priority,
    pub due_date: Option<NaiveDate>,
    pub parent: Option<u64>,
    #[serde(default)]
    pub labels: Vec<String>,
    pub assignee: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTask {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub priority: Option<Priority>,
    pub due_date: Option<NaiveDate>,
    pub parent: Option<u64>,
    pub labels: Option<Vec<String>>,
    pub archived: Option<bool>,
    pub assignee: Option<String>,
}

fn default_status() -> TaskStatus {
    TaskStatus::Todo
}
fn default_priority() -> Priority {
    Priority::Medium
}

// --- Enums ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Todo,
    InProgress,
    InReview,
    Done,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Todo => write!(f, "todo"),
            TaskStatus::InProgress => write!(f, "in_progress"),
            TaskStatus::InReview => write!(f, "in_review"),
            TaskStatus::Done => write!(f, "done"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Priority::Low => write!(f, "low"),
            Priority::Medium => write!(f, "medium"),
            Priority::High => write!(f, "high"),
            Priority::Critical => write!(f, "critical"),
        }
    }
}

// --- Comments ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: u64,
    pub project_key: String,
    pub task_id: u64,
    pub author: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateComment {
    #[serde(default = "default_author")]
    pub author: String,
    pub body: String,
}

fn default_author() -> String {
    "Anonymous".into()
}

// --- Activity Log ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub id: u64,
    pub project_key: String,
    pub task_id: Option<u64>,
    pub action: String,
    pub field: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub actor: String,
    pub created_at: DateTime<Utc>,
}

// --- Query Params ---

#[derive(Debug, Deserialize)]
pub struct TaskQuery {
    #[serde(default = "default_false")]
    pub archived: Option<bool>,
    #[serde(default = "default_task_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

fn default_false() -> Option<bool> {
    Some(false)
}

fn default_task_limit() -> u64 {
    100
}

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    #[serde(default = "default_activity_limit")]
    pub limit: u64,
}

fn default_activity_limit() -> u64 {
    50
}

// --- Bulk Operations ---

#[derive(Debug, Deserialize)]
pub struct BulkUpdateTasks {
    pub task_ids: Vec<u64>,
    pub status: Option<TaskStatus>,
    pub archived: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct BulkDeleteTasks {
    pub task_ids: Vec<u64>,
}

// --- Dashboard ---

#[derive(Debug, Serialize, Deserialize)]
pub struct DashboardData {
    pub status_counts: StatusCounts,
    pub overdue_count: u64,
    pub upcoming_tasks: Vec<Task>,
    pub recently_updated: Vec<Task>,
    pub velocity: Vec<WeeklyVelocity>,
    pub burndown: Vec<BurndownPoint>,
    pub label_distribution: Vec<LabelCount>,
    pub overdue_tasks_detail: Vec<OverdueTaskDetail>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusCounts {
    pub todo: u64,
    pub in_progress: u64,
    pub in_review: u64,
    pub done: u64,
    pub total: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WeeklyVelocity {
    pub week: String,
    pub count: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BurndownPoint {
    pub date: String,
    pub remaining: i64,
    pub completed: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LabelCount {
    pub label: String,
    pub count: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OverdueTaskDetail {
    pub id: u64,
    pub key: String,
    pub title: String,
    pub due_date: String,
    pub days_overdue: i64,
    pub priority: String,
}

// --- Experiment ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomPalette {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidebar_bg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_primary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_bg: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackEntry {
    pub participant_id: String,
    pub variant: String,
    pub rating: u8,
    pub preferred_variant: String,
    pub comments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_palette: Option<CustomPalette>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct SubmitFeedback {
    pub rating: u8,
    pub preferred_variant: String,
    #[serde(default)]
    pub comments: String,
    #[serde(default)]
    pub custom_palette: Option<CustomPalette>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentAssignment {
    pub participant_id: String,
    pub variant: String,
    pub assigned_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct VariantSummary {
    pub variant: String,
    pub feedback_count: u64,
    pub avg_rating: f64,
    pub preferred_count: u64,
    pub comments: Vec<String>,
}

// --- CO-45: UAT mutation tracking ---

/// A single write operation recorded in the `uat_mutations` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UatMutation {
    pub id: i64,
    pub timestamp: String,
    pub user_id: Option<String>,
    /// e.g. "entry.create", "entry.update", "entry.delete"
    pub action: String,
    /// "{universe_key}:{entry_path}" for entry mutations
    pub target: String,
    pub before_value: Option<String>,
    pub after_value: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExperimentSummary {
    pub total_feedback: u64,
    pub variants: Vec<VariantSummary>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VariantResponse {
    pub variant: String,
}

#[derive(Debug, Deserialize)]
pub struct SwitchVariant {
    pub variant: String,
}

// --- Health ---

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub env: String,
}

#[derive(Debug, Serialize)]
pub struct HealthDeepResponse {
    pub status: String,
    pub db: String,
    pub disk: String,
}

// --- Universe / Multi-tenancy ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Universe {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub owner_id: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub is_template: bool,
    #[serde(default)]
    pub is_public: bool,
    #[serde(default)]
    pub content_count: i64,
    /// CO-38: if true, anonymous visitors cannot access this universe.
    #[serde(default)]
    pub requires_login: bool,
    /// CO-49: single visibility enum replacing is_public + is_template + requires_login.
    /// Values: "template", "private", "public-subscribable", "requires_login"
    #[serde(default = "default_visibility")]
    pub visibility: String,
    /// CO-98: optional parent universe key for hierarchical grouping in the
    /// sidebar (e.g. timeline trio under `template`). `None` = top-level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_key: Option<String>,
    /// CO-330: when true, anonymous reads are restricted to entries with `published: true`
    /// in their frontmatter. Owner/authenticated callers see all entries.
    #[serde(default)]
    pub anon_published_only: bool,
    /// CO-383: origin kind for event-bus-backed universes ('event-bus' | 'remote-git' | 'local-only').
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,
    /// CO-383: event bus URL (e.g. wss://yggdrasil.artelonga.com.br/api/v1/events).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// CO-383: ISO timestamp of the last event received from the upstream bus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_last_event_at: Option<String>,
    /// CO-413: write policy for event-bus-backed universes — `'read-only'`
    /// (default) or `'bidirectional'`. When bidirectional, CO accepts writes and
    /// re-emits them to the federated bus as CO-origin edits. `None` on pre-v83
    /// rows (treated as read-only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_mode: Option<String>,
    /// CO-338: deployment DNS host — set only on deployable units (e.g.
    /// `yggdrasil.artelonga.com.br`). `None` = the universe inherits a deploying
    /// ancestor's DNS. Drives `key::path` surface-ref resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_dns: Option<String>,
    /// CO-89: opt-in git source for this universe (e.g.
    /// `github.com/artelonga/co.git`). When set, a background job ingests the
    /// repo's `git log` as `commit`/`profile`/`event` content entries. `None` =
    /// markdown-only universe (behaves exactly as before CO-89).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_source: Option<String>,
    /// CO-89: branch ingested by git-sync. Defaults to `main`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    /// CO-89: SHA of the last commit imported — the incremental-sync cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_last_synced_sha: Option<String>,
    /// CO-89: ISO-8601 timestamp of the last successful git-sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_last_synced_at: Option<String>,
    /// CO-93: opt-in flag that turns a public-subscribable (private) universe
    /// into a **private-dynamic** one — subscribers may submit pending edits via
    /// the proposal flow. Default `false` (the universe is static for its read
    /// audience). The canonical [`Universe::universe_type`] is derived from
    /// `visibility` + this flag; there is no denormalized `universe_type` column.
    #[serde(default)]
    pub accepts_proposals: bool,
}

fn default_visibility() -> String {
    "private".into()
}

/// CO-93: the three first-class universe types (plus the system-owned `template`
/// flavor of public-static). This is the canonical, user-facing taxonomy that
/// unifies the orthogonal axes previously conflated under `visibility`:
/// **read visibility** (public vs private), **edit model** (members write
/// directly), **proposal model** (only `private-dynamic` accepts subscriber
/// proposals), and **encryption-at-rest** (on for private types).
///
/// The type is *derived* deterministically from `visibility` + `accepts_proposals`
/// — see [`Universe::universe_type`] for the mapping — so it stays consistent
/// with the legacy column instead of drifting like the old
/// `is_template`/`is_public`/`requires_login` trio did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UniverseType {
    /// Read by anyone (no auth); members edit directly; no proposals; no
    /// encryption; pre-renderable to a CDN.
    PublicStatic,
    /// Read by members only (auth required); members edit directly; no
    /// proposals; encrypted at rest.
    PrivateStatic,
    /// Read by members + subscribers; members edit directly; subscribers submit
    /// proposals via the review queue; encrypted at rest.
    PrivateDynamic,
    /// System-owned public-static (read anyone, write system-only, no human
    /// owner) — the `template` universe.
    Template,
}

impl UniverseType {
    /// Canonical kebab-case wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            UniverseType::PublicStatic => "public-static",
            UniverseType::PrivateStatic => "private-static",
            UniverseType::PrivateDynamic => "private-dynamic",
            UniverseType::Template => "template",
        }
    }

    /// Whether content of this type is encrypted at rest on the server
    /// (ciphertext-only). True for the two private types.
    pub fn encrypted_at_rest(self) -> bool {
        matches!(
            self,
            UniverseType::PrivateStatic | UniverseType::PrivateDynamic
        )
    }

    /// Whether anonymous (no-auth) reads are allowed. True for the public types.
    pub fn public_read(self) -> bool {
        matches!(self, UniverseType::PublicStatic | UniverseType::Template)
    }

    /// Whether subscribers may submit proposals (only `private-dynamic`).
    pub fn accepts_proposals(self) -> bool {
        matches!(self, UniverseType::PrivateDynamic)
    }

    /// Whether the read path can be pre-rendered and cached at a CDN edge
    /// (public types only — private content must never hit a shared cache).
    pub fn static_exportable(self) -> bool {
        self.public_read()
    }
}

impl std::fmt::Display for UniverseType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Universe {
    /// CO-93: derive the canonical [`UniverseType`] from the legacy `visibility`
    /// column plus the `accepts_proposals` opt-in flag. This is the single source
    /// of truth for the three-types taxonomy — see the "Today → after CO-93"
    /// mapping:
    ///
    /// | `visibility`                       | `accepts_proposals` | `universe_type`  |
    /// |------------------------------------|---------------------|------------------|
    /// | `template`                         | (any)               | `template`       |
    /// | `public-subscribable`              | `true`              | `private-dynamic`|
    /// | `public-subscribable`              | `false`             | `public-static`  |
    /// | `private`                          | (any)               | `private-static` |
    /// | `requires_login`                   | (any)               | `private-static` |
    /// | anything else (unknown/legacy)     | (any)               | `private-static` |
    ///
    /// `requires_login` is subsumed: any logged-in user is treated as a member of
    /// a login-gated universe, so it collapses to `private-static`.
    pub fn universe_type(&self) -> UniverseType {
        match self.visibility.as_str() {
            "template" => UniverseType::Template,
            "public-subscribable" if self.accepts_proposals => UniverseType::PrivateDynamic,
            "public-subscribable" => UniverseType::PublicStatic,
            // `private`, `requires_login`, and any unknown/legacy value all map to
            // the safe private-static default (fail closed).
            _ => UniverseType::PrivateStatic,
        }
    }
}

/// CO-191: Universe with resolved role for the requesting user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseWithRole {
    #[serde(flatten)]
    pub universe: Universe,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// CO-191: Response for GET /api/v1/me/universes — all universe relationships
/// bucketed by type. Order within each bucket: by name.
#[derive(Debug, Serialize)]
pub struct MeUniversesResponse {
    pub owned: Vec<UniverseWithRole>,
    pub member: Vec<UniverseWithRole>,
    pub subscribed: Vec<UniverseWithRole>,
    pub invited: Vec<crate::invitation_routes::MeInvitationItem>,
    pub discoverable: Vec<UniverseWithRole>,
    pub counts: MeUniversesCounts,
}

#[derive(Debug, Serialize)]
pub struct MeUniversesCounts {
    pub owned: usize,
    pub member: usize,
    pub subscribed: usize,
    pub invited: usize,
    pub discoverable: usize,
}

/// CO-49: Deterministic access level for a universe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UniverseAccess {
    /// Full read + write access (owner or member with write role).
    ReadWrite,
    /// Read-only access (member with read role, subscriber, or any logged-in user for requires_login universes).
    ReadOnly,
    /// Only public metadata visible (title, description, subscriber count).
    MetadataOnly,
    /// Login required; return 401 (universe exists but is not accessible anonymously).
    LoginRequired,
    /// Universe not accessible; return 404.
    Denied,
}

/// CO-49: A user's subscription to a public-subscribable universe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub user_id: String,
    pub universe_key: String,
    pub subscribed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseMember {
    pub universe_key: String,
    pub user_id: String,
    pub role: String,
    pub joined_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateUniverse {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct CloneUniverse {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct AddMember {
    pub user_id: String,
    #[serde(default = "default_member_role")]
    pub role: String,
}

fn default_member_role() -> String {
    "member".into()
}

// --- Universe Form Config ---

/// Presentation config for a universe — drives CSS, layout, and fonts.
/// Stored in `universes.theme_preset/layout/...` and synced to `.universo.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseFormConfig {
    /// CSS preset name: scholarly-light, scholarly-dark, relic, relic-light
    #[serde(default = "default_theme_preset")]
    pub theme_preset: String,
    /// Default view mode: board, table, timeline, calendar, dashboard
    #[serde(default = "default_layout")]
    pub layout: String,
    /// Custom headline font (Google Fonts family name), if any
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_headline: Option<String>,
    /// Custom body font (Google Fonts family name), if any
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_body: Option<String>,
    /// CSS token overrides, e.g. `{"--color-accent":"#ff0000"}`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_tokens: Option<serde_json::Value>,
}

fn default_theme_preset() -> String {
    "scholarly-light".to_string()
}
fn default_layout() -> String {
    "board".to_string()
}

impl Default for UniverseFormConfig {
    fn default() -> Self {
        Self {
            theme_preset: default_theme_preset(),
            layout: default_layout(),
            font_headline: None,
            font_body: None,
            custom_tokens: None,
        }
    }
}

/// Partial update payload for `PUT /api/v1/universes/:slug/config`.
#[derive(Debug, Default, Deserialize)]
pub struct UpdateUniverseFormConfig {
    pub theme_preset: Option<String>,
    pub layout: Option<String>,
    pub font_headline: Option<String>,
    pub font_body: Option<String>,
    pub custom_tokens: Option<serde_json::Value>,
}

// --- Theme Tiers ---

/// Available themes returned by `GET /api/v1/themes/available`.
/// Content depends on whether the caller is a real logged-in user or anonymous.
#[derive(Debug, Serialize)]
pub struct AvailableThemes {
    /// Named palette keys available to this user.
    pub palettes: Vec<String>,
    /// Variant keys (a–h) available to this user. Empty for anonymous.
    pub variants: Vec<String>,
    /// Whether the custom palette editor is available. Absent for anonymous.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<bool>,
}

// --- Auth / Users ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeResponse {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub tier: String,
    /// CO-173: list of universes the user has any relation to (owner, member,
    /// or subscriber), each with a metadata bag pulled from the source-of-
    /// truth for that universe.
    /// Defaults to empty so older clients that only read user fields keep
    /// working.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub universes: Vec<UserUniverseEntry>,
    /// CO-198: DM privacy policy (everyone | shared-universe | nobody).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dm_policy: Option<String>,
    /// CO-370: linked lead id (NULL if user has no lead record).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_id: Option<i64>,
    /// CO-370: status of the linked lead (new | triaged | in_progress | closed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_status: Option<String>,
    /// CO-370: acquisition channel that created the lead (lead_form | signup | invitation | manual).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_source: Option<String>,
}

/// CO-173: per-universe metadata bag for the authenticated user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserUniverseEntry {
    pub key: String,
    pub name: String,
    /// Best-effort role string. For CO universes: `"owner"` / `"admin"` / `"editor"` / `"viewer"`.
    pub role: String,
    pub is_owner: bool,
    pub is_member: bool,
    pub is_subscriber: bool,
    /// Universe-source-specific metadata. Default CO universe: `{joined_at}`.
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub tier: String,
    pub created_at: DateTime<Utc>,
    pub usuario: Option<String>,
}

// --- CO-165: Recovery channels ---

#[derive(Debug, Clone)]
pub struct RecoveryChannel {
    pub id: String,
    pub user_id: String,
    pub channel_type: String,
    pub value_ciphertext: Vec<u8>,
    pub value_nonce: [u8; 12],
    pub value_lookup_hash: String,
    pub verified_at: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub lockout_until: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RecoveryVerification {
    pub id: String,
    pub channel_id: String,
    pub user_id: String,
    pub purpose: String,
    pub code_hash: String,
    pub expires_at: String,
    pub consumed_at: Option<String>,
    pub attempts: i64,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct PasswordResetToken {
    pub token_hash: String,
    pub user_id: String,
    pub channel_id: String,
    pub expires_at: String,
    pub consumed_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct RecoveryChannelResponse {
    pub id: String,
    pub channel_type: String,
    pub masked_value: String,
    pub verified_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub message: String,
    /// CO-303: populated in non-prod envs (`is_local_or_test()`) so developers
    /// can complete magic-code login through the UI without email delivery.
    /// Never set in production.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dev_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VerifyRequest {
    pub email: String,
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub expires_at: DateTime<Utc>,
}

#[cfg(test)]
mod universe_type_tests {
    use super::*;

    /// Minimal `Universe` fixture parameterised on the two axes that drive the
    /// CO-93 type derivation: `visibility` + `accepts_proposals`.
    fn universe(visibility: &str, accepts_proposals: bool) -> Universe {
        Universe {
            key: "k".into(),
            name: "K".into(),
            description: String::new(),
            owner_id: "u1".into(),
            created_at: Utc::now(),
            is_template: visibility == "template",
            is_public: false,
            content_count: 0,
            requires_login: visibility == "requires_login",
            visibility: visibility.into(),
            parent_key: None,
            anon_published_only: false,
            source_kind: None,
            source_url: None,
            source_last_event_at: None,
            source_mode: None,
            surface_dns: None,
            git_source: None,
            git_branch: None,
            git_last_synced_sha: None,
            git_last_synced_at: None,
            accepts_proposals,
        }
    }

    /// CO-93: every row of the "Today → after CO-93" mapping table resolves to
    /// the canonical type. This is the single source of truth for the taxonomy.
    #[test]
    fn derives_each_universe_type_from_visibility_and_flag() {
        // private → private-static (the legacy default)
        assert_eq!(
            universe("private", false).universe_type(),
            UniverseType::PrivateStatic
        );
        // public-subscribable, no proposals → public-static
        assert_eq!(
            universe("public-subscribable", false).universe_type(),
            UniverseType::PublicStatic
        );
        // public-subscribable, proposals opted in → private-dynamic
        assert_eq!(
            universe("public-subscribable", true).universe_type(),
            UniverseType::PrivateDynamic
        );
        // requires_login is subsumed by membership → private-static
        assert_eq!(
            universe("requires_login", false).universe_type(),
            UniverseType::PrivateStatic
        );
        // template stays a special public-static flavor (even if a stray flag is set)
        assert_eq!(
            universe("template", true).universe_type(),
            UniverseType::Template
        );
        // unknown / legacy values fail closed → private-static
        assert_eq!(
            universe("something-weird", false).universe_type(),
            UniverseType::PrivateStatic
        );
    }

    /// CO-93: `accepts_proposals` only promotes `public-subscribable` to dynamic.
    /// It must NOT turn a `private` or `template` universe into private-dynamic.
    #[test]
    fn accepts_proposals_only_promotes_public_subscribable() {
        assert_eq!(
            universe("private", true).universe_type(),
            UniverseType::PrivateStatic,
            "a private universe with the flag set is still private-static"
        );
        assert_eq!(
            universe("template", true).universe_type(),
            UniverseType::Template,
            "the system template is never private-dynamic"
        );
    }

    /// CO-93: the type's capability accessors encode the access-model matrix —
    /// encryption-at-rest, anonymous read, proposal flow, CDN exportability.
    #[test]
    fn type_capabilities_match_the_access_matrix() {
        // public types: anonymous read + static-exportable, never encrypted.
        for t in [UniverseType::PublicStatic, UniverseType::Template] {
            assert!(t.public_read());
            assert!(t.static_exportable());
            assert!(!t.encrypted_at_rest());
            assert!(!t.accepts_proposals());
        }
        // private types: encrypted at rest, no anonymous read, not exportable.
        for t in [UniverseType::PrivateStatic, UniverseType::PrivateDynamic] {
            assert!(t.encrypted_at_rest());
            assert!(!t.public_read());
            assert!(!t.static_exportable());
        }
        // only private-dynamic accepts proposals.
        assert!(UniverseType::PrivateDynamic.accepts_proposals());
        assert!(!UniverseType::PrivateStatic.accepts_proposals());
    }

    /// CO-93: canonical kebab-case wire strings (serde + `as_str`/`Display`).
    #[test]
    fn wire_strings_are_canonical_kebab_case() {
        assert_eq!(UniverseType::PublicStatic.as_str(), "public-static");
        assert_eq!(UniverseType::PrivateStatic.as_str(), "private-static");
        assert_eq!(UniverseType::PrivateDynamic.as_str(), "private-dynamic");
        assert_eq!(UniverseType::Template.as_str(), "template");
        assert_eq!(UniverseType::PrivateDynamic.to_string(), "private-dynamic");
        // serde round-trips through the same kebab-case representation.
        let json = serde_json::to_string(&UniverseType::PrivateDynamic).unwrap();
        assert_eq!(json, "\"private-dynamic\"");
        let back: UniverseType = serde_json::from_str("\"public-static\"").unwrap();
        assert_eq!(back, UniverseType::PublicStatic);
    }
}
