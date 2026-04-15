use super::types::CarrierRequestStatus;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CarrierStatusTransitionError {
    #[error("transition from {from} to {to} is not allowed")]
    NotAllowed {
        from: &'static str,
        to: &'static str,
    },

    #[error("cannot transition from terminal status {0}")]
    Terminal(&'static str),
}

/// Validate a carrier request status transition.
///
/// Valid transitions:
///   pending   → submitted
///   submitted → completed | failed
///   failed    → submitted (retry)
pub fn validate_carrier_status(
    from: CarrierRequestStatus,
    to: CarrierRequestStatus,
) -> Result<(), CarrierStatusTransitionError> {
    if from.is_terminal() {
        return Err(CarrierStatusTransitionError::Terminal(from.as_str()));
    }

    let allowed = matches!(
        (from, to),
        (
            CarrierRequestStatus::Pending,
            CarrierRequestStatus::Submitted
        ) | (
            CarrierRequestStatus::Submitted,
            CarrierRequestStatus::Completed
        ) | (
            CarrierRequestStatus::Submitted,
            CarrierRequestStatus::Failed
        ) | (
            CarrierRequestStatus::Failed,
            CarrierRequestStatus::Submitted
        )
    );

    if !allowed {
        return Err(CarrierStatusTransitionError::NotAllowed {
            from: from.as_str(),
            to: to.as_str(),
        });
    }

    Ok(())
}
