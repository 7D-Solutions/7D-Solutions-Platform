use super::types::DocRequestStatus;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DocStatusTransitionError {
    #[error("transition from {from} to {to} is not allowed")]
    NotAllowed {
        from: &'static str,
        to: &'static str,
    },

    #[error("cannot transition from terminal status {0}")]
    Terminal(&'static str),
}

/// Validate a doc request status transition.
///
/// Valid transitions:
///   requested  → generating
///   generating → completed | failed
///   failed     → generating (retry)
pub fn validate_doc_status(
    from: DocRequestStatus,
    to: DocRequestStatus,
) -> Result<(), DocStatusTransitionError> {
    if from.is_terminal() {
        return Err(DocStatusTransitionError::Terminal(from.as_str()));
    }

    let allowed = matches!(
        (from, to),
        (DocRequestStatus::Requested, DocRequestStatus::Generating)
            | (DocRequestStatus::Generating, DocRequestStatus::Completed)
            | (DocRequestStatus::Generating, DocRequestStatus::Failed)
            | (DocRequestStatus::Failed, DocRequestStatus::Generating)
    );

    if !allowed {
        return Err(DocStatusTransitionError::NotAllowed {
            from: from.as_str(),
            to: to.as_str(),
        });
    }

    Ok(())
}
