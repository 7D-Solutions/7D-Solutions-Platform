# ADR-{NUMBER}: {TITLE}

**Date:** YYYY-MM-DD  
**Status:** [Proposed | Accepted | Deprecated | Superseded]  
**Deciders:** [List of people involved]  
**Technical Story:** [Link to issue/story]

## Context and Problem Statement

[Describe the context and background. What is the issue you're trying to address? What forces are at play? What architectural concerns exist?]

[Keep this section factual and objective. State the problem clearly without proposing solutions yet.]

## Decision Drivers

* [Driver 1: e.g., "Need to support 10,000 concurrent users"]
* [Driver 2: e.g., "Must maintain backward compatibility"]
* [Driver 3: e.g., "Limited engineering resources"]
* [Driver 4: e.g., "Security compliance requirements"]

## Considered Options

* [Option 1: Brief title]
* [Option 2: Brief title]
* [Option 3: Brief title]

## Decision Outcome

Chosen option: "[Option X]", because [justification summary].

### Positive Consequences

* [Benefit 1]
* [Benefit 2]
* [Benefit 3]

### Negative Consequences

* [Downside 1 and mitigation]
* [Downside 2 and mitigation]
* [Downside 3 and mitigation]

## Detailed Analysis

### Option 1: [Title]

[Describe the option in detail]

**Pros:**
* [Advantage 1]
* [Advantage 2]

**Cons:**
* [Disadvantage 1]
* [Disadvantage 2]

**Cost/Effort:** [Estimate: Low/Medium/High]

**Risk:** [Level: Low/Medium/High]

### Option 2: [Title]

[Describe the option in detail]

**Pros:**
* [Advantage 1]
* [Advantage 2]

**Cons:**
* [Disadvantage 1]
* [Disadvantage 2]

**Cost/Effort:** [Estimate: Low/Medium/High]

**Risk:** [Level: Low/Medium/High]

### Option 3: [Title]

[Describe the option in detail]

**Pros:**
* [Advantage 1]
* [Advantage 2]

**Cons:**
* [Disadvantage 1]
* [Disadvantage 2]

**Cost/Effort:** [Estimate: Low/Medium/High]

**Risk:** [Level: Low/Medium/High]

## Implementation Plan

### Phase 1: [Name]
**Timeline:** [Estimate]
**Deliverables:**
* [Deliverable 1]
* [Deliverable 2]

### Phase 2: [Name]
**Timeline:** [Estimate]
**Deliverables:**
* [Deliverable 1]
* [Deliverable 2]

### Phase 3: [Name]
**Timeline:** [Estimate]
**Deliverables:**
* [Deliverable 1]
* [Deliverable 2]

## Validation Criteria

How will we know this decision was correct?

* [Metric 1: e.g., "Response time < 100ms"]
* [Metric 2: e.g., "Zero data loss incidents"]
* [Metric 3: e.g., "Developer onboarding time < 2 days"]

**Review Date:** [Date to revisit this decision]

## Links

* [Link to related ADR]
* [Link to technical spike]
* [Link to proof of concept]
* [Link to external resources]

---

## Example ADR

# ADR-001: Use NATS for Event Bus

**Date:** 2026-02-11  
**Status:** Accepted  
**Deciders:** James (Platform Architect), Engineering Team  
**Technical Story:** Issue #42 - Choose event bus technology

## Context and Problem Statement

The 7D Solutions Platform requires an event bus for asynchronous communication between modules. Modules must be able to publish domain events (e.g., "invoice.created", "customer.updated") and subscribe to events from other modules without tight coupling.

We need a solution that is:
- Lightweight enough for small deployments
- Scalable to enterprise workloads
- Cloud-native and containerizable
- Simple to operate
- Rust-friendly (first-class client support)

## Decision Drivers

* Need pub/sub messaging between modules
* Must support JetStream (persistence, replay)
* Prefer Rust-native client library
* Must run on-premise and in cloud
* Low operational overhead
* Built-in observability

## Considered Options

* Option 1: NATS JetStream
* Option 2: Apache Kafka
* Option 3: RabbitMQ

## Decision Outcome

Chosen option: "NATS JetStream", because it provides the best balance of simplicity, performance, and Rust support while meeting all functional requirements.

### Positive Consequences

* Lightweight: Single binary, ~20MB memory footprint
* Excellent Rust client: async-nats crate
* Built-in persistence and replay (JetStream)
* Simple operations: No Zookeeper, no complex config
* Cloud-native: Kubernetes-friendly

### Negative Consequences

* Less mature ecosystem than Kafka
* Smaller community
* Mitigation: NATS is CNCF project with strong backing

## Detailed Analysis

### Option 1: NATS JetStream

Cloud-native messaging system with JetStream for persistence.

**Pros:**
* Lightweight: ~20MB RAM, single binary
* Excellent performance: 10M+ msgs/sec
* First-class Rust support (async-nats)
* JetStream provides persistence, replay
* Built-in observability (Prometheus)
* Simple operations (no dependencies)

**Cons:**
* Smaller ecosystem vs Kafka
* Fewer enterprise adoptions
* Less tooling around monitoring

**Cost/Effort:** Low (simple deployment)

**Risk:** Low (CNCF project, proven at scale)

### Option 2: Apache Kafka

Distributed streaming platform, industry standard.

**Pros:**
* Industry standard
* Massive ecosystem
* Battle-tested at scale
* Rich tooling

**Cons:**
* Heavy: Requires Zookeeper (Kafka < 3.3)
* Complex operations
* High resource usage
* Rust client less mature
* Overkill for our initial scale

**Cost/Effort:** High (complex deployment)

**Risk:** Medium (operational complexity)

### Option 3: RabbitMQ

Traditional message broker with AMQP protocol.

**Pros:**
* Mature, stable
* Good Rust client (lapin)
* Flexible routing
* Good management UI

**Cons:**
* Less performant than NATS/Kafka
* Erlang-based (less familiar to team)
* Not as cloud-native
* Persistence less robust than JetStream

**Cost/Effort:** Medium

**Risk:** Medium (performance concerns at scale)

## Implementation Plan

### Phase 1: Deploy NATS
**Timeline:** 1 week
**Deliverables:**
* NATS JetStream running in Docker Compose
* Basic health checks
* Prometheus metrics

### Phase 2: Platform Integration
**Timeline:** 2 weeks
**Deliverables:**
* Event publishing library
* Event subscription framework
* Dead letter queue handling

### Phase 3: Module Migration
**Timeline:** Ongoing
**Deliverables:**
* Migrate modules to use event bus
* Remove point-to-point integrations

## Validation Criteria

* Throughput: Handle 10,000 events/sec
* Latency: P99 < 100ms end-to-end
* Reliability: Zero message loss
* Operations: Single-person team can manage

**Review Date:** 2026-08-11 (6 months)

## Links

* [NATS Documentation](https://docs.nats.io/)
* [JetStream Guide](https://docs.nats.io/nats-concepts/jetstream)
* [async-nats crate](https://docs.rs/async-nats/)
* [Platform Event Bus Design](./PLATFORM-EVENTS.md)
