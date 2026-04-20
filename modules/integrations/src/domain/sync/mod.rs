pub mod authority;
pub mod authority_repo;
pub mod authority_service;
pub mod conflicts;
pub mod conflicts_repo;
pub mod dedupe;
pub mod detector;
pub mod health;
pub mod observations;
pub mod push_attempts;
pub mod resolve_customer;
pub mod resolve_invoice;
pub mod resolve_payment;
pub mod resolve_service;

pub use authority::{AuthorityRow, AuthoritySide};
pub use authority_service::{flip_authority, FlipError, FlipResult};
pub use conflicts::{ConflictClass, ConflictError, ConflictRow, ConflictStatus, MAX_VALUE_BYTES};
pub use dedupe::{compute_comparable_hash, compute_fingerprint, compute_resolve_det_key, truncate_to_millis};
pub use detector::{run_detector, DetectorError, DetectorOutcome};
pub use observations::ObservationRow;
pub use push_attempts::{
    post_call_reconcile, pre_call_version_check, PreCallOutcome, PushAttemptRow, PushStatus,
    ReconcileError, ReconcileOutcome,
};
