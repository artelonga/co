use serde::{Deserialize, Serialize};

// ---- Request types ----

/// Legacy login (kept for compatibility -- Godot client)
#[derive(Deserialize)]
pub struct LegacyLoginRequest {
    pub name: String,
}

/// Step 1: request a verification code by email
#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
}

/// Step 2: verify the code and receive a JWT
#[derive(Deserialize)]
pub struct VerifyRequest {
    pub email: String,
    pub code: String,
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    pub display_name: String,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Deserialize)]
pub struct GameResultRequest {
    pub score: i64,
}

#[derive(Deserialize)]
pub struct LeaderboardQuery {
    pub limit: Option<u32>,
}

// ---- Response types ----

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Serialize)]
pub struct PluginListEntry {
    pub name: String,
    pub version: String,
    pub description: String,
}

/// Returned after a successful verify (step 2)
#[derive(Serialize, Deserialize)]
pub struct VerifyResponse {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub expires_at: i64,
}

/// Returned when wrong code is supplied
#[derive(Serialize, Deserialize)]
pub struct WrongCodeResponse {
    pub error: String,
    pub remaining_attempts: u32,
}

/// Legacy response (Godot client)
#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user_id: String,
    pub display_name: String,
}

#[derive(Serialize)]
pub struct ProfileResponse {
    pub user_id: String,
    pub display_name: String,
    pub created_at: i64,
    pub last_login_at: i64,
    pub stats: Option<StatsResponse>,
}

#[derive(Serialize)]
pub struct StatsResponse {
    pub hands_played: u64,
    pub hands_won: u64,
    pub total_chips_won: u64,
    pub total_chips_lost: u64,
    pub biggest_pot_won: u64,
    pub best_hand_rank: u32,
    pub sessions_played: u64,
}

#[derive(Serialize, Deserialize)]
pub struct WalletResponse {
    pub balance: u64,
}

#[derive(Serialize, Deserialize)]
pub struct GameStatsResponse {
    pub game_name: String,
    pub high_score: i64,
    pub games_played: u64,
    pub last_played_at: i64,
}

#[derive(Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub rank: u32,
    pub user_id: String,
    pub display_name: String,
    pub username: String,
    pub high_score: i64,
    pub games_played: u64,
}

#[derive(Serialize, Deserialize)]
pub struct PlayerProfileResponse {
    pub user_id: String,
    pub username: String,
    pub display_name: String,
    pub created_at: i64,
    pub game_stats: Vec<GameStatsResponse>,
    pub universes: Vec<UniverseEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct UniverseEntry {
    pub name: String,
    pub description: String,
}

#[derive(Serialize, Deserialize)]
pub struct RegisterResponse {
    pub user_id: String,
    pub username: String,
    pub display_name: String,
}

#[derive(Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Serialize, Deserialize)]
pub struct ValidationErrorResponse {
    pub error: String,
    pub fields: Vec<FieldError>,
}

#[derive(Serialize, Deserialize)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

/// Entry in the global leaderboard (sum of high scores across all games).
#[derive(Serialize, Deserialize)]
pub struct GlobalLeaderboardEntry {
    pub rank: u32,
    pub user_id: String,
    pub display_name: String,
    pub username: String,
    pub total_score: i64,
    pub games_played: u64,
}

/// A single recent-activity event in the Yggdrasil feed.
#[derive(Serialize, Deserialize)]
pub struct RecentActivityEntry {
    pub user_id: String,
    pub display_name: String,
    pub username: String,
    pub game_name: String,
    pub score: i64,
    pub played_at: i64,
}

// ---- Internal structures stored as JSON in redb ----

#[derive(Serialize, Deserialize)]
pub struct VerifyCodeData {
    pub code: String,
    pub user_id: String,
    pub expires_at: i64,
    pub attempts: u32,
}

#[derive(Serialize, Deserialize)]
pub struct RateLimitData {
    pub count: u32,
    pub window_start: i64,
}
