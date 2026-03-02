//! Stable NATS subjects for the workflow module.
//!
//! Changing a subject is a **breaking change** — bump MAJOR version.

pub const INSTANCE_STARTED: &str = "workflow.events.instance.started";
pub const INSTANCE_ADVANCED: &str = "workflow.events.instance.advanced";
pub const INSTANCE_COMPLETED: &str = "workflow.events.instance.completed";
pub const INSTANCE_CANCELLED: &str = "workflow.events.instance.cancelled";
pub const DEFINITION_CREATED: &str = "workflow.events.definition.created";

pub const ALL_SUBJECTS: &[&str] = &[
    INSTANCE_STARTED,
    INSTANCE_ADVANCED,
    INSTANCE_COMPLETED,
    INSTANCE_CANCELLED,
    DEFINITION_CREATED,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_subjects_start_with_workflow() {
        for subject in ALL_SUBJECTS {
            assert!(
                subject.starts_with("workflow.events."),
                "Subject '{}' must start with 'workflow.events.'",
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
        assert_eq!(ALL_SUBJECTS.len(), 5);
    }
}
