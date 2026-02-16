/// Audit actor (who performed the action)
///
/// Placeholder for actor identification and authentication context.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {
    pub id: Uuid,
    pub actor_type: ActorType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActorType {
    User,
    Service,
    System,
}
