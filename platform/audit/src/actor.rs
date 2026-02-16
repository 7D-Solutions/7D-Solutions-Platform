/// Audit actor (who performed the action)
///
/// Actors represent the identity performing an action in the system.
/// Every audited mutation must have a non-empty actor identity.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Actor {
    pub id: Uuid,
    pub actor_type: ActorType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActorType {
    /// End user (authenticated via API key, JWT, etc.)
    User,
    /// Internal service actor (background jobs, schedulers)
    Service,
    /// System-level operations (migrations, maintenance)
    System,
}

impl Actor {
    /// Create a new user actor
    pub fn user(id: Uuid) -> Self {
        Self {
            id,
            actor_type: ActorType::User,
        }
    }

    /// Create a new service actor with a deterministic ID
    pub fn service(service_name: &str) -> Self {
        // Use a deterministic UUID v5 based on service name
        // This ensures the same service always has the same actor ID
        let namespace = Uuid::NAMESPACE_OID;
        let id = Uuid::new_v5(&namespace, service_name.as_bytes());
        Self {
            id,
            actor_type: ActorType::Service,
        }
    }

    /// Create a new system actor
    pub fn system() -> Self {
        // Use a well-known UUID for system operations
        Self {
            id: Uuid::nil(),
            actor_type: ActorType::System,
        }
    }

    /// Get the actor type as a string for database storage
    pub fn actor_type_str(&self) -> String {
        match self.actor_type {
            ActorType::User => "User".to_string(),
            ActorType::Service => "Service".to_string(),
            ActorType::System => "System".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_actor() {
        let user_id = Uuid::new_v4();
        let actor = Actor::user(user_id);
        assert_eq!(actor.id, user_id);
        assert_eq!(actor.actor_type, ActorType::User);
        assert_eq!(actor.actor_type_str(), "User");
    }

    #[test]
    fn test_service_actor_deterministic() {
        let actor1 = Actor::service("billing-scheduler");
        let actor2 = Actor::service("billing-scheduler");
        assert_eq!(actor1.id, actor2.id, "Service actors should have deterministic IDs");
        assert_eq!(actor1.actor_type, ActorType::Service);
        assert_eq!(actor1.actor_type_str(), "Service");
    }

    #[test]
    fn test_service_actor_unique_per_service() {
        let actor1 = Actor::service("billing-scheduler");
        let actor2 = Actor::service("notification-worker");
        assert_ne!(actor1.id, actor2.id, "Different services should have different actor IDs");
    }

    #[test]
    fn test_system_actor() {
        let actor = Actor::system();
        assert_eq!(actor.id, Uuid::nil());
        assert_eq!(actor.actor_type, ActorType::System);
        assert_eq!(actor.actor_type_str(), "System");
    }
}
