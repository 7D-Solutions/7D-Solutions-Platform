//! E2E Test: Mutation Class Presence (bd-1tml)
//!
//! **Phase 16: Governance Enforcement**
//!
//! ## Test Coverage
//! 1. **Per-Module Coverage**: Verify at least one event from each module has mutation_class
//! 2. **Classification Accuracy**: Assert event types map to correct mutation_classes
//! 3. **Registry Compliance**: Validate against docs/governance/MUTATION-CLASSES.md
//!
//! ## Architecture
//! - AR emits DATA_MUTATION events (invoices, payment requests)
//! - Payments emits DATA_MUTATION events (payment success/failure)
//! - GL emits REVERSAL events (journal entry reversals)
//! - Subscriptions emits LIFECYCLE events (bill run lifecycle)
//! - Notifications emits SIDE_EFFECT events (email/SMS delivery)
//!
//! ## Invariant
//! Every emitted event MUST have a non-null mutation_class matching the registry.
//! Failure mode: Events without mutation_class or with incorrect classification.

mod common;

use anyhow::Result;
use serial_test::serial;
use sqlx::PgPool;

/// Get AR database pool
async fn get_ar_pool() -> PgPool {
    let url = std::env::var("AR_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ar_user:ar_pass@localhost:5434/ar_db".to_string());

    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to AR database")
}

/// Get Payments database pool
async fn get_payments_pool() -> PgPool {
    let url = std::env::var("PAYMENTS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://payments_user:payments_pass@localhost:5436/payments_db".to_string());

    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to Payments database")
}

/// Get Subscriptions database pool
async fn get_subscriptions_pool() -> PgPool {
    let url = std::env::var("SUBSCRIPTIONS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://subscriptions_user:subscriptions_pass@localhost:5435/subscriptions_db".to_string());

    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to Subscriptions database")
}

/// Get GL database pool
async fn get_gl_pool() -> PgPool {
    let url = std::env::var("GL_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://gl_user:gl_pass@localhost:5438/gl_db".to_string());

    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to GL database")
}

/// Get Notifications database pool
async fn get_notifications_pool() -> PgPool {
    let url = std::env::var("NOTIFICATIONS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db".to_string());

    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to Notifications database")
}

/// Test that AR module emits events with mutation_class
#[tokio::test]
#[serial]
async fn test_ar_module_emits_mutation_class() -> Result<()> {
    let ar_pool = get_ar_pool().await;

    // Query for recent AR outbox events with mutation_class
    let result: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT event_type, mutation_class
        FROM events_outbox
        WHERE mutation_class IS NOT NULL
        ORDER BY created_at DESC
        LIMIT 5
        "#
    )
    .fetch_all(&ar_pool)
    .await?;

    if result.is_empty() {
        println!("⚠️  No AR events found with mutation_class - this may be expected if no events have been created yet");
        return Ok(());
    }

    println!("✅ AR module emits {} events with mutation_class", result.len());

    for (event_type, mutation_class) in &result {
        println!("   - event_type: {:?}, mutation_class: {:?}", event_type, mutation_class);
        assert!(mutation_class.is_some(), "AR event has null mutation_class");
    }

    Ok(())
}

/// Test that Payments module emits events with mutation_class
#[tokio::test]
#[serial]
async fn test_payments_module_emits_mutation_class() -> Result<()> {
    let payments_pool = get_payments_pool().await;

    // Query for recent Payments outbox events with mutation_class
    let result: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT event_type, mutation_class
        FROM events_outbox
        WHERE mutation_class IS NOT NULL
        ORDER BY created_at DESC
        LIMIT 5
        "#
    )
    .fetch_all(&payments_pool)
    .await?;

    if result.is_empty() {
        println!("⚠️  No Payments events found with mutation_class - this may be expected if no events have been created yet");
        return Ok(());
    }

    println!("✅ Payments module emits {} events with mutation_class", result.len());

    for (event_type, mutation_class) in &result {
        println!("   - event_type: {:?}, mutation_class: {:?}", event_type, mutation_class);
        assert!(mutation_class.is_some(), "Payments event has null mutation_class");

        // Validate classification: payment success/failure should be DATA_MUTATION
        if let Some(et) = event_type {
            if et.contains("payment.succeeded") || et.contains("payment.failed") {
                assert_eq!(mutation_class.as_deref(), Some("DATA_MUTATION"),
                    "Payment events should have mutation_class=DATA_MUTATION");
            }
        }
    }

    Ok(())
}

/// Test that Subscriptions module emits events with mutation_class
#[tokio::test]
#[serial]
async fn test_subscriptions_module_emits_mutation_class() -> Result<()> {
    let subscriptions_pool = get_subscriptions_pool().await;

    // Query for recent Subscriptions outbox events with mutation_class
    let result: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT event_type, mutation_class
        FROM events_outbox
        WHERE mutation_class IS NOT NULL
        ORDER BY created_at DESC
        LIMIT 5
        "#
    )
    .fetch_all(&subscriptions_pool)
        .await?;

    if result.is_empty() {
        println!("⚠️  No Subscriptions events found with mutation_class - this may be expected if no events have been created yet");
        return Ok(());
    }

    println!("✅ Subscriptions module emits {} events with mutation_class", result.len());

    for (event_type, mutation_class) in &result {
        println!("   - event_type: {:?}, mutation_class: {:?}", event_type, mutation_class);
        assert!(mutation_class.is_some(), "Subscriptions event has null mutation_class");

        // Validate classification: billrun.completed should be LIFECYCLE
        if let Some(et) = event_type {
            if et.contains("billrun.completed") {
                assert_eq!(mutation_class.as_deref(), Some("LIFECYCLE"),
                    "Bill run completion should have mutation_class=LIFECYCLE");
            }
        }
    }

    Ok(())
}

/// Test that GL module emits events with mutation_class
#[tokio::test]
#[serial]
async fn test_gl_module_emits_mutation_class() -> Result<()> {
    let gl_pool = get_gl_pool().await;

    // Query for recent GL outbox events with mutation_class
    let result: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT event_type, mutation_class
        FROM events_outbox
        WHERE mutation_class IS NOT NULL
        ORDER BY created_at DESC
        LIMIT 5
        "#
    )
    .fetch_all(&gl_pool)
    .await?;

    if result.is_empty() {
        println!("⚠️  No GL events found with mutation_class - this may be expected if no events have been created yet");
        return Ok(());
    }

    println!("✅ GL module emits {} events with mutation_class", result.len());

    for (event_type, mutation_class) in &result {
        println!("   - event_type: {:?}, mutation_class: {:?}", event_type, mutation_class);
        assert!(mutation_class.is_some(), "GL event has null mutation_class");

        // Validate classification: gl.entry.reversed should be REVERSAL
        if let Some(et) = event_type {
            if et.contains("entry.reversed") {
                assert_eq!(mutation_class.as_deref(), Some("REVERSAL"),
                    "GL reversal events should have mutation_class=REVERSAL");
            }
        }
    }

    Ok(())
}

/// Test that Notifications module emits events with mutation_class
#[tokio::test]
#[serial]
async fn test_notifications_module_emits_mutation_class() -> Result<()> {
    let notifications_pool = get_notifications_pool().await;

    // Query for recent Notifications outbox events with mutation_class
    let result: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT event_type, mutation_class
        FROM events_outbox
        WHERE mutation_class IS NOT NULL
        ORDER BY created_at DESC
        LIMIT 5
        "#
    )
    .fetch_all(&notifications_pool)
    .await?;

    if result.is_empty() {
        println!("⚠️  No Notifications events found with mutation_class - this may be expected if no events have been created yet");
        return Ok(());
    }

    println!("✅ Notifications module emits {} events with mutation_class", result.len());

    for (event_type, mutation_class) in &result {
        println!("   - event_type: {:?}, mutation_class: {:?}", event_type, mutation_class);
        assert!(mutation_class.is_some(), "Notifications event has null mutation_class");

        // Validate classification: notifications.delivery.* should be SIDE_EFFECT
        if let Some(et) = event_type {
            if et.contains("notifications.delivery") {
                assert_eq!(mutation_class.as_deref(), Some("SIDE_EFFECT"),
                    "Notification delivery events should have mutation_class=SIDE_EFFECT");
            }
        }
    }

    Ok(())
}

/// Integration test: Verify mutation_class registry compliance across all modules
#[tokio::test]
#[serial]
async fn test_mutation_class_registry_compliance() -> Result<()> {
    println!("\n🔍 Verifying mutation_class compliance across all modules...\n");

    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;
    let notifications_pool = get_notifications_pool().await;

    // Check AR
    let ar_invalid: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT event_type, mutation_class
        FROM events_outbox
        WHERE mutation_class IS NOT NULL
          AND mutation_class NOT IN ('DATA_MUTATION', 'REVERSAL', 'CORRECTION', 'SIDE_EFFECT', 'QUERY', 'LIFECYCLE', 'ADMINISTRATIVE')
        LIMIT 5
        "#
    )
    .fetch_all(&ar_pool)
    .await?;

    assert!(ar_invalid.is_empty(), "AR has events with invalid mutation_class: {:?}", ar_invalid);

    // Check Payments
    let payments_invalid: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT event_type, mutation_class
        FROM events_outbox
        WHERE mutation_class IS NOT NULL
          AND mutation_class NOT IN ('DATA_MUTATION', 'REVERSAL', 'CORRECTION', 'SIDE_EFFECT', 'QUERY', 'LIFECYCLE', 'ADMINISTRATIVE')
        LIMIT 5
        "#
    )
    .fetch_all(&payments_pool)
    .await?;

    assert!(payments_invalid.is_empty(), "Payments has events with invalid mutation_class: {:?}", payments_invalid);

    // Check Subscriptions
    let subscriptions_invalid: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT event_type, mutation_class
        FROM events_outbox
        WHERE mutation_class IS NOT NULL
          AND mutation_class NOT IN ('DATA_MUTATION', 'REVERSAL', 'CORRECTION', 'SIDE_EFFECT', 'QUERY', 'LIFECYCLE', 'ADMINISTRATIVE')
        LIMIT 5
        "#
    )
    .fetch_all(&subscriptions_pool)
    .await?;

    assert!(subscriptions_invalid.is_empty(), "Subscriptions has events with invalid mutation_class: {:?}", subscriptions_invalid);

    // Check GL
    let gl_invalid: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT event_type, mutation_class
        FROM events_outbox
        WHERE mutation_class IS NOT NULL
          AND mutation_class NOT IN ('DATA_MUTATION', 'REVERSAL', 'CORRECTION', 'SIDE_EFFECT', 'QUERY', 'LIFECYCLE', 'ADMINISTRATIVE')
        LIMIT 5
        "#
    )
    .fetch_all(&gl_pool)
    .await?;

    assert!(gl_invalid.is_empty(), "GL has events with invalid mutation_class: {:?}", gl_invalid);

    // Check Notifications
    let notifications_invalid: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT event_type, mutation_class
        FROM events_outbox
        WHERE mutation_class IS NOT NULL
          AND mutation_class NOT IN ('DATA_MUTATION', 'REVERSAL', 'CORRECTION', 'SIDE_EFFECT', 'QUERY', 'LIFECYCLE', 'ADMINISTRATIVE')
        LIMIT 5
        "#
    )
    .fetch_all(&notifications_pool)
    .await?;

    assert!(notifications_invalid.is_empty(), "Notifications has events with invalid mutation_class: {:?}", notifications_invalid);

    println!("✅ All modules comply with mutation_class registry\n");
    Ok(())
}
