use super::*;

// --- Template guard ---

/// Returns Forbidden if the given project belongs to a template (read-only) universe.
pub(super) fn guard_template(state: &AppState, project_key: &str) -> Result<(), AppError> {
    if lock_storage(state)?.is_project_in_template(project_key) {
        return Err(AppError::Forbidden("Template universe is read-only".into()));
    }
    Ok(())
}

/// Check whether a universe has hit the anonymous usage limit (100 entries).
/// Returns Ok(()) if allowed, Err(AppError::UsageLimitExceeded) if blocked.
pub(super) fn check_usage_gate(
    storage: &crate::storage::Storage,
    universe_key: &str,
) -> Result<(), AppError> {
    let Some(universe) = storage.get_universe(universe_key) else {
        return Ok(()); // Unknown universe — let it through (other validation will catch it)
    };
    if universe.owner_id.starts_with("anon-") && universe.content_count >= 100 {
        return Err(AppError::UsageLimitExceeded {
            current: universe.content_count,
        });
    }
    Ok(())
}

// --- Project Handlers ---

pub(super) async fn list_projects(
    State(state): State<AppState>,
    user_id: crate::auth::UserId,
) -> Result<Json<Vec<Project>>, AppError> {
    let storage = lock_storage(&state)?;
    let projects = storage.list_projects_for_user(&user_id.0);
    Ok(Json(projects))
}

pub(super) async fn get_project(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<Project>, AppError> {
    let storage = lock_storage(&state)?;
    storage
        .get_project(&key)
        .map(Json)
        .ok_or_else(|| AppError::NotFound(format!("Project '{}' not found", key)))
}

pub(super) async fn create_project(
    State(state): State<AppState>,
    Json(mut body): Json<CreateProject>,
) -> Result<impl IntoResponse, AppError> {
    validate_project_name(&body.name)?;
    validate_project_key(&body.key)?;

    // Prevent creating projects inside the template universe.
    if body.universe_key.as_deref() == Some("template") {
        return Err(AppError::Forbidden("Template universe is read-only".into()));
    }

    // Server-side universe scope takes precedence over client-supplied value.
    if state.config.universe_key.is_some() {
        body.universe_key = state.config.universe_key.clone();
    }

    let mut storage = lock_storage(&state)?;

    // Check usage gate for universe-scoped projects.
    if let Some(ref ukey) = body.universe_key {
        check_usage_gate(&storage, ukey)?;
    }

    // Capture the universe_key before consuming body.
    let universe_key = body.universe_key.clone();

    let project = storage
        .create_project(body)
        .map_err(|e| AppError::Conflict(e.to_string()))?;

    // Increment content_count for the universe.
    if let Some(ref ukey) = universe_key {
        storage.increment_universe_content_count(ukey);
    }

    Ok((StatusCode::CREATED, Json(project)))
}

pub(super) async fn delete_project(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<StatusCode, AppError> {
    guard_template(&state, &key)?;
    let mut storage = lock_storage(&state)?;

    let universe_key = storage.get_project_universe_key(&key);
    let project_content = universe_key
        .as_deref()
        .map(|_| storage.count_project_content(&key))
        .unwrap_or(0);

    storage
        .delete_project(&key)
        .map_err(|_| AppError::NotFound(format!("Project '{}' not found", key)))?;

    // Decrement: 1 for the project itself + tasks + their comments
    if let Some(ref ukey) = universe_key {
        storage.decrement_universe_content_count(ukey, 1 + project_content);
    }

    Ok(StatusCode::NO_CONTENT)
}

// --- Task Handlers ---

pub(super) async fn list_tasks(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<TaskQuery>,
) -> Result<Json<Vec<Task>>, AppError> {
    let limit = query.limit.min(500);
    let storage = lock_storage(&state)?;
    Ok(Json(storage.list_tasks_paginated(
        &key,
        query.archived,
        limit,
        query.offset,
    )))
}

pub(super) async fn get_task(
    State(state): State<AppState>,
    Path((key, id)): Path<(String, u64)>,
) -> Result<Json<Task>, AppError> {
    let storage = lock_storage(&state)?;
    storage
        .get_task(&key, id)
        .map(Json)
        .ok_or_else(|| AppError::NotFound(format!("Task {}-{} not found", key, id)))
}

pub(super) async fn create_task(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<CreateTask>,
) -> Result<impl IntoResponse, AppError> {
    guard_template(&state, &key)?;
    validate_task_title(&body.title)?;
    validate_task_description(&body.description)?;
    validate_labels(&body.labels)?;

    let mut storage = lock_storage(&state)?;

    // Check usage gate.
    if let Some(ukey) = storage.get_project_universe_key(&key) {
        check_usage_gate(&storage, &ukey)?;
        let task = storage
            .create_task(&key, body)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        storage.increment_universe_content_count(&ukey);
        return Ok((StatusCode::CREATED, Json(task)));
    }

    storage
        .create_task(&key, body)
        .map(|t| (StatusCode::CREATED, Json(t)))
        .map_err(|e| AppError::Internal(e.to_string()))
}

pub(super) async fn update_task(
    State(state): State<AppState>,
    Path((key, id)): Path<(String, u64)>,
    Json(body): Json<UpdateTask>,
) -> Result<Json<Task>, AppError> {
    guard_template(&state, &key)?;
    if let Some(ref title) = body.title {
        validate_task_title(title)?;
    }
    if let Some(ref description) = body.description {
        validate_task_description(description)?;
    }
    if let Some(ref labels) = body.labels {
        validate_labels(labels)?;
    }

    let mut storage = lock_storage(&state)?;
    storage
        .update_task(&key, id, body)
        .map(Json)
        .map_err(|e| AppError::Internal(e.to_string()))
}

pub(super) async fn delete_task(
    State(state): State<AppState>,
    Path((key, id)): Path<(String, u64)>,
) -> Result<StatusCode, AppError> {
    guard_template(&state, &key)?;
    let mut storage = lock_storage(&state)?;

    let universe_key = storage.get_project_universe_key(&key);
    let comment_count = universe_key
        .as_deref()
        .map(|_| storage.count_task_comments(&key, id))
        .unwrap_or(0);

    storage
        .delete_task(&key, id)
        .map_err(|e| AppError::NotFound(e.to_string()))?;

    if let Some(ref ukey) = universe_key {
        storage.decrement_universe_content_count(ukey, 1 + comment_count);
    }

    Ok(StatusCode::NO_CONTENT)
}

// --- Comment Handlers ---

pub(super) async fn list_comments(
    State(state): State<AppState>,
    Path((key, id)): Path<(String, u64)>,
) -> Result<Json<Vec<Comment>>, AppError> {
    let storage = lock_storage(&state)?;
    Ok(Json(storage.list_comments(&key, id)))
}

pub(super) async fn create_comment(
    State(state): State<AppState>,
    Path((key, id)): Path<(String, u64)>,
    Json(body): Json<CreateComment>,
) -> Result<impl IntoResponse, AppError> {
    guard_template(&state, &key)?;
    validate_comment_body(&body.body)?;
    validate_comment_author(&body.author)?;

    let mut storage = lock_storage(&state)?;

    // Check usage gate.
    if let Some(ukey) = storage.get_project_universe_key(&key) {
        check_usage_gate(&storage, &ukey)?;
        let comment = storage
            .create_comment(&key, id, body)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        storage.increment_universe_content_count(&ukey);
        return Ok((StatusCode::CREATED, Json(comment)));
    }

    storage
        .create_comment(&key, id, body)
        .map(|c| (StatusCode::CREATED, Json(c)))
        .map_err(|e| AppError::Internal(e.to_string()))
}

// --- Activity Handler ---

pub(super) async fn list_activity(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<Vec<ActivityEntry>>, AppError> {
    let limit = query.limit.min(200);
    let storage = lock_storage(&state)?;
    Ok(Json(storage.list_activity(&key, limit)))
}

// --- Dashboard Handler ---

pub(super) async fn get_dashboard(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<DashboardData>, AppError> {
    let storage = lock_storage(&state)?;
    Ok(Json(storage.get_dashboard(&key)))
}

// --- Bulk Operations ---

pub(super) async fn bulk_update_tasks(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<BulkUpdateTasks>,
) -> Result<Json<Vec<Task>>, AppError> {
    guard_template(&state, &key)?;
    if body.task_ids.is_empty() {
        return Err(AppError::BadRequest("task_ids cannot be empty".into()));
    }

    let mut storage = lock_storage(&state)?;
    storage
        .bulk_update_tasks(&key, body)
        .map(Json)
        .map_err(|e| AppError::Internal(e.to_string()))
}

pub(super) async fn bulk_delete_tasks(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<BulkDeleteTasks>,
) -> Result<StatusCode, AppError> {
    guard_template(&state, &key)?;
    let mut storage = lock_storage(&state)?;
    storage
        .bulk_delete_tasks(&key, body)
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| AppError::Internal(e.to_string()))
}
