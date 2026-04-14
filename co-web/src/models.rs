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
}

fn default_visibility() -> String {
    "private".into()
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

// --- Universe Git config (CO-50) ---

/// CO-50: Git repository config for a universe (as stored in the DB).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseGitConfig {
    pub universe_key: String,
    /// GitHub repo slug: `"owner/repo"`
    pub repo: String,
    /// Optional subdirectory within the repo, e.g. `"data/co"`
    pub path: Option<String>,
    /// Branch to track (default: `"main"`)
    pub branch: String,
    /// Last successfully synced commit hash
    pub commit_hash: Option<String>,
    /// When the last sync completed
    pub synced_at: Option<DateTime<Utc>>,
    /// Last sync error message, if any
    pub sync_error: Option<String>,
}

/// CO-50: Request body to configure a git-backed universe.
#[derive(Debug, Deserialize)]
pub struct SetGitConfig {
    /// GitHub repo slug: `"owner/repo"`
    pub repo: String,
    /// Optional subdirectory within the repo
    pub path: Option<String>,
    /// Branch to track (default: `"main"`)
    #[serde(default = "default_git_branch")]
    pub branch: String,
}

fn default_git_branch() -> String {
    "main".into()
}

/// CO-50: GitHub webhook push payload (subset we care about).
#[derive(Debug, Deserialize)]
pub struct GithubWebhookPush {
    /// The ref that was pushed to, e.g. `"refs/heads/main"`
    #[serde(rename = "ref")]
    pub git_ref: String,
    /// The new HEAD commit hash after the push
    pub after: Option<String>,
}

// --- Theme Tiers ---

/// Stats for the `quilomboaraucaria` public universe.
///
/// Counts are derived from the SQLite entry index; totalUsuarios, versaoApp,
/// and ultimaSync come from the `meta/stats.md` metadata entry written by the
/// importer on each run.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuilomboStats {
    pub total_usuarios: i64,
    pub total_publicacoes: i64,
    pub total_eventos: i64,
    pub total_missoes: i64,
    pub versao_app: String,
    pub ultima_sync: Option<String>,
}

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub tier: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub message: String,
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
