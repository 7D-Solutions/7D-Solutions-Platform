//! Benchmarks for EventEnvelope creation and serialization.
//!
//! Run with: cargo bench -p event-bus --bench envelope_bench

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use event_bus::EventEnvelope;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TestPayload {
    item_id: String,
    quantity: i32,
    location_id: String,
}

fn bench_envelope_creation(c: &mut Criterion) {
    c.bench_function("envelope_new", |b| {
        b.iter(|| {
            let envelope = EventEnvelope::new(
                black_box("tenant-12345678-1234-1234-1234-123456789012".to_string()),
                black_box("inventory".to_string()),
                black_box("inventory.item_issued".to_string()),
                TestPayload {
                    item_id: "item-001".to_string(),
                    quantity: 100,
                    location_id: "loc-001".to_string(),
                },
            );
            black_box(envelope)
        })
    });
}

fn bench_envelope_with_builders(c: &mut Criterion) {
    c.bench_function("envelope_with_all_builders", |b| {
        b.iter(|| {
            let envelope = EventEnvelope::new(
                black_box("tenant-12345678-1234-1234-1234-123456789012".to_string()),
                black_box("inventory".to_string()),
                black_box("inventory.item_issued".to_string()),
                TestPayload {
                    item_id: "item-001".to_string(),
                    quantity: 100,
                    location_id: "loc-001".to_string(),
                },
            )
            .with_source_version("1.2.3")
            .with_schema_version("1.0.0")
            .with_correlation_id(Some("corr-001".to_string()))
            .with_causation_id(Some("cause-001".to_string()))
            .with_mutation_class(Some("inventory".to_string()))
            .with_actor(Uuid::new_v4(), "user");
            black_box(envelope)
        })
    });
}

fn bench_envelope_serialization(c: &mut Criterion) {
    let envelope = EventEnvelope::new(
        "tenant-12345678-1234-1234-1234-123456789012".to_string(),
        "inventory".to_string(),
        "inventory.item_issued".to_string(),
        TestPayload {
            item_id: "item-001".to_string(),
            quantity: 100,
            location_id: "loc-001".to_string(),
        },
    )
    .with_source_version("1.2.3")
    .with_schema_version("1.0.0")
    .with_correlation_id(Some("corr-001".to_string()))
    .with_causation_id(Some("cause-001".to_string()));

    c.bench_function("envelope_serialize_json", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&envelope)).unwrap();
            black_box(json)
        })
    });

    let json = serde_json::to_string(&envelope).unwrap();
    c.bench_function("envelope_deserialize_json", |b| {
        b.iter(|| {
            let env: EventEnvelope<TestPayload> =
                serde_json::from_str(black_box(&json)).unwrap();
            black_box(env)
        })
    });
}

fn bench_envelope_clone(c: &mut Criterion) {
    let envelope = EventEnvelope::new(
        "tenant-12345678-1234-1234-1234-123456789012".to_string(),
        "inventory".to_string(),
        "inventory.item_issued".to_string(),
        TestPayload {
            item_id: "item-001".to_string(),
            quantity: 100,
            location_id: "loc-001".to_string(),
        },
    )
    .with_source_version("1.2.3")
    .with_schema_version("1.0.0")
    .with_correlation_id(Some("corr-001".to_string()))
    .with_causation_id(Some("cause-001".to_string()));

    c.bench_function("envelope_clone", |b| {
        b.iter(|| {
            let cloned = black_box(&envelope).clone();
            black_box(cloned)
        })
    });
}

criterion_group!(
    benches,
    bench_envelope_creation,
    bench_envelope_with_builders,
    bench_envelope_serialization,
    bench_envelope_clone,
);
criterion_main!(benches);
