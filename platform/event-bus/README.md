# EventBus — Platform-Level Messaging Abstraction

**Tier 1 Infrastructure** | **Version 0.1.0**

## Overview

The EventBus is a platform-level abstraction for event-driven messaging across all modules in the 7D Solutions Platform. It provides a unified interface for publishing and subscribing to events, with pluggable implementations for different environments.

## Why This Lives in Tier 1 (Platform)

The EventBus is a **shared runtime capability** that all modules depend on. By placing it in `platform/` rather than a shared module:

1. **Avoids circular dependencies** - Modules can depend on platform crates without creating cycles
2. **Enables plug-and-play modules** - Each module gets the bus from the platform, not from other modules
3. **Config-driven swapping** - Switch between NATS (production) and InMemory (dev/test) with zero code changes
4. **Follows architecture standards** - Tier 1 provides foundational capabilities to Tier 2 modules

## Implementations

### NatsBus (Production)

Uses NATS JetStream for durable, distributed messaging:
- Persistent storage
- At-least-once delivery
- Multi-tenant support via stream isolation
- Monitoring and observability

### InMemoryBus (Dev/Test)

Uses Tokio broadcast channels for fast, in-process messaging:
- Zero external dependencies
- Instant startup (no Docker required)
- Perfect for unit tests
- Deterministic message ordering

## Usage

### Basic Publishing

```rust
use event_bus::{EventBus, InMemoryBus, NatsBus};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Development: In-Memory
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());

    // Production: NATS
    // let nats_client = async_nats::connect("nats://localhost:4222").await?;
    // let bus: Arc<dyn EventBus> = Arc::new(NatsBus::new(nats_client));

    // Publish an event
    let payload = serde_json::to_vec(&serde_json::json!({
        "event_type": "user.created",
        "user_id": "usr_123",
        "timestamp": "2026-02-11T23:00:00Z"
    }))?;

    bus.publish("auth.events.user.created", payload).await?;

    Ok(())
}
```

### Subscribing to Events

```rust
use event_bus::{EventBus, InMemoryBus};
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bus = InMemoryBus::new();

    // Subscribe to all auth events
    let mut stream = bus.subscribe("auth.events.>").await?;

    // Process events as they arrive
    while let Some(msg) = stream.next().await {
        println!("Received event on {}: {} bytes",
            msg.subject, msg.payload.len());

        // Deserialize and process
        let event: serde_json::Value = serde_json::from_slice(&msg.payload)?;
        println!("Event: {:?}", event);
    }

    Ok(())
}
```

### Subject Pattern Matching

The EventBus supports NATS-style wildcards:

- **`*`** - Matches exactly one token
  - `auth.*.created` matches `auth.user.created`
  - `auth.*.created` does NOT match `auth.user.profile.created`

- **`>`** - Matches one or more tokens (must be last)
  - `auth.events.>` matches `auth.events.user.created`
  - `auth.events.>` matches `auth.events.user.profile.updated`
  - `auth.>` matches any subject starting with `auth.`

### Config-Driven Swap

```rust
use event_bus::{EventBus, NatsBus, InMemoryBus};
use std::sync::Arc;

async fn create_bus(config: &AppConfig) -> Result<Arc<dyn EventBus>, Box<dyn std::error::Error>> {
    let bus: Arc<dyn EventBus> = match config.bus_type {
        BusType::Nats => {
            let client = async_nats::connect(&config.nats_url).await?;
            Arc::new(NatsBus::new(client))
        }
        BusType::InMemory => {
            Arc::new(InMemoryBus::new())
        }
    };

    Ok(bus)
}
```

## Module Integration

### Adding EventBus to a Module

1. **Add dependency** to `Cargo.toml`:
   ```toml
   [dependencies]
   event-bus = { path = "../../platform/event-bus" }
   ```

2. **Wire into application state**:
   ```rust
   use event_bus::EventBus;
   use std::sync::Arc;

   pub struct AppState {
       pub bus: Arc<dyn EventBus>,
       // ... other state
   }
   ```

3. **Publish events** from handlers:
   ```rust
   async fn create_user(
       State(state): State<Arc<AppState>>,
       Json(payload): Json<CreateUserRequest>,
   ) -> Result<Json<UserResponse>, AppError> {
       // ... create user in database

       // Publish event
       let event = UserCreatedEvent { user_id, email, ... };
       let event_bytes = serde_json::to_vec(&event)?;
       state.bus.publish("auth.events.user.created", event_bytes).await?;

       Ok(Json(response))
   }
   ```

4. **Subscribe to events** in background tasks:
   ```rust
   use futures::StreamExt;

   async fn start_event_consumer(bus: Arc<dyn EventBus>) {
       let mut stream = bus.subscribe("auth.events.>").await.unwrap();

       while let Some(msg) = stream.next().await {
           match handle_event(&msg).await {
               Ok(_) => log::info!("Processed event: {}", msg.subject),
               Err(e) => log::error!("Failed to process event: {}", e),
           }
       }
   }
   ```

## Testing

### Unit Tests with InMemoryBus

```rust
#[cfg(test)]
mod tests {
    use event_bus::{EventBus, InMemoryBus};
    use futures::StreamExt;

    #[tokio::test]
    async fn test_user_creation_publishes_event() {
        let bus = Arc::new(InMemoryBus::new());
        let mut stream = bus.subscribe("auth.events.user.created").await.unwrap();

        // Execute the operation
        let state = AppState { bus: bus.clone() };
        create_user(state, ...).await.unwrap();

        // Verify event was published
        let msg = tokio::time::timeout(
            Duration::from_secs(1),
            stream.next()
        ).await.unwrap().unwrap();

        let event: UserCreatedEvent = serde_json::from_slice(&msg.payload).unwrap();
        assert_eq!(event.user_id, "usr_123");
    }
}
```

### Integration Tests with NATS

For full integration tests, use docker-compose to spin up NATS:

```yaml
# docker-compose.test.yml
services:
  nats:
    image: nats:2.10-alpine
    command: ["-js", "-sd", "/data"]
    ports:
      - "4222:4222"
```

Then in your test:

```rust
#[tokio::test]
#[ignore] // Requires docker-compose
async fn test_with_real_nats() {
    let client = async_nats::connect("nats://localhost:4222").await.unwrap();
    let bus = Arc::new(NatsBus::new(client));

    // Test with real NATS...
}
```

## Metrics and Observability

Future work (Phase 5.x):
- [ ] Add metrics hooks to EventBus trait (messages published, subscribers, errors)
- [ ] Integrate with OpenTelemetry for distributed tracing
- [ ] Add dead letter queue (DLQ) support for failed message processing
- [ ] Stream health checks and monitoring endpoints

## Architecture Decisions

### Why Not Use NATS Directly?

**Testability**: Direct NATS usage requires Docker in every test environment
**Flexibility**: Abstraction allows swapping backends (NATS → Kafka, Pulsar) without module changes
**Local Development**: Developers can run the entire platform without external dependencies
**Contract Testing**: InMemoryBus enables deterministic contract tests

### Why Trait Object Instead of Generic?

```rust
// This would be hard to use:
pub struct AppState<B: EventBus> {
    pub bus: Arc<B>,
}

// This is easier:
pub struct AppState {
    pub bus: Arc<dyn EventBus>,
}
```

Using `Arc<dyn EventBus>` simplifies dependency injection and avoids generic complexity throughout the application.

## Performance Characteristics

### InMemoryBus
- **Latency**: Sub-microsecond (in-process channels)
- **Throughput**: 1M+ msg/sec on modern hardware
- **Overhead**: Minimal (no serialization over network)
- **Durability**: None (messages lost on restart)

### NatsBus
- **Latency**: 1-5ms (local network), 50-200ms (cross-region)
- **Throughput**: 10K-100K msg/sec (depends on persistence config)
- **Overhead**: Network + disk I/O
- **Durability**: Configurable (memory, file, replicated)

## Next Steps for Phase 5

1. **Outbox Publisher** (Bead 5.1) - Publish events from database outbox table
2. **Event Consumers** (Bead 5.2) - Module-specific event handlers
3. **Idempotency** (Bead 5.3) - Ensure exactly-once processing semantics
4. **Stream Setup** (Bead 5.4) - JetStream stream initialization and schema registry

All of these will be dramatically simpler now that the Bus trait is locked.

## References

- [NATS Documentation](https://docs.nats.io/)
- [JetStream Architecture](https://docs.nats.io/nats-concepts/jetstream)
- [AR Module Spec](../../AR-MODULE-SPEC.md) - Example of event-driven module design
- [Event Contracts](../../contracts/events/) - Platform event schemas
