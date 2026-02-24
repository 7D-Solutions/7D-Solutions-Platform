//! Stable NATS subjects for the maintenance module.
//!
//! These subjects form the public event contract. Downstream consumers
//! (notifications, reporting, integrations) subscribe to these subjects.
//! Changing a subject is a **breaking change** — bump MAJOR version.

// ── Work Order lifecycle ─────────────────────────────────────
pub const WO_CREATED: &str = "maintenance.work_order.created";
pub const WO_STATUS_CHANGED: &str = "maintenance.work_order.status_changed";
pub const WO_COMPLETED: &str = "maintenance.work_order.completed";
pub const WO_CLOSED: &str = "maintenance.work_order.closed";
pub const WO_CANCELLED: &str = "maintenance.work_order.cancelled";
pub const WO_OVERDUE: &str = "maintenance.work_order.overdue";

// ── Meter readings ───────────────────────────────────────────
pub const METER_READING_RECORDED: &str = "maintenance.meter_reading.recorded";

// ── Plan lifecycle ───────────────────────────────────────────
pub const PLAN_DUE: &str = "maintenance.plan.due";
pub const PLAN_ASSIGNED: &str = "maintenance.plan.assigned";

/// All stable NATS subjects exported by the maintenance module.
///
/// Used in tests to assert exhaustive coverage and in documentation.
pub const ALL_SUBJECTS: &[&str] = &[
    WO_CREATED,
    WO_STATUS_CHANGED,
    WO_COMPLETED,
    WO_CLOSED,
    WO_CANCELLED,
    WO_OVERDUE,
    METER_READING_RECORDED,
    PLAN_DUE,
    PLAN_ASSIGNED,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_subjects_start_with_maintenance() {
        for subject in ALL_SUBJECTS {
            assert!(
                subject.starts_with("maintenance."),
                "Subject '{}' must start with 'maintenance.'",
                subject
            );
        }
    }

    #[test]
    fn no_duplicate_subjects() {
        let mut seen = std::collections::HashSet::new();
        for subject in ALL_SUBJECTS {
            assert!(seen.insert(subject), "Duplicate subject: {}", subject);
        }
    }

    #[test]
    fn all_subjects_count() {
        assert_eq!(ALL_SUBJECTS.len(), 9);
    }
}
