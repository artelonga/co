use crate::error::AppError;

pub fn validate_task_title(title: &str) -> Result<(), AppError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("Task title cannot be empty".into()));
    }
    if title.len() > 500 {
        return Err(AppError::BadRequest(
            "Task title must be 500 characters or fewer".into(),
        ));
    }
    Ok(())
}

pub fn validate_task_description(desc: &str) -> Result<(), AppError> {
    if desc.len() > 10_000 {
        return Err(AppError::BadRequest(
            "Task description must be 10,000 characters or fewer".into(),
        ));
    }
    Ok(())
}

pub fn validate_comment_body(body: &str) -> Result<(), AppError> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("Comment body cannot be empty".into()));
    }
    if body.len() > 5_000 {
        return Err(AppError::BadRequest(
            "Comment body must be 5,000 characters or fewer".into(),
        ));
    }
    Ok(())
}

pub fn validate_comment_author(author: &str) -> Result<(), AppError> {
    if author.len() > 100 {
        return Err(AppError::BadRequest(
            "Comment author must be 100 characters or fewer".into(),
        ));
    }
    Ok(())
}

pub fn validate_project_name(name: &str) -> Result<(), AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("Project name cannot be empty".into()));
    }
    if name.len() > 100 {
        return Err(AppError::BadRequest(
            "Project name must be 100 characters or fewer".into(),
        ));
    }
    Ok(())
}

pub fn validate_project_key(key: &str) -> Result<(), AppError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("Project key cannot be empty".into()));
    }
    if key.len() > 10 {
        return Err(AppError::BadRequest(
            "Project key must be 10 characters or fewer".into(),
        ));
    }
    if !key.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::BadRequest(
            "Project key must contain only alphanumeric characters".into(),
        ));
    }
    Ok(())
}

pub fn validate_labels(labels: &[String]) -> Result<(), AppError> {
    if labels.len() > 20 {
        return Err(AppError::BadRequest("Maximum 20 labels allowed".into()));
    }
    for label in labels {
        if label.len() > 50 {
            return Err(AppError::BadRequest(
                "Each label must be 50 characters or fewer".into(),
            ));
        }
    }
    Ok(())
}
