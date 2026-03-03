use thiserror::Error;

#[derive(Debug, Error)]
pub enum GuardError {
    #[error("Validation: {0}")]
    Validation(String),

    #[error("Artifact not found or wrong tenant")]
    ArtifactNotFound,

    #[error("Artifact is inactive")]
    ArtifactInactive,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub fn guard_non_empty(value: &str, field: &str) -> Result<(), GuardError> {
    if value.trim().is_empty() {
        return Err(GuardError::Validation(format!("{field} is required")));
    }
    Ok(())
}
