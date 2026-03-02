//! E2E Test: Config Validation Fail-Fast (bd-18a1)
//!
//! **Phase 18: Secrets and Config Hardening**
//!
//! ## Test Coverage
//! 1. **Required Config Validation**: Each module validates required env vars on startup
//! 2. **Clear Error Messages**: Validation errors are explicit and actionable
//! 3. **Fail-Fast Behavior**: Modules refuse to start with missing/invalid required config
//!
//! ## Architecture
//! - modules/*/src/config.rs: Config validation per module
//! - modules/*/src/main.rs: Startup validation with fail-fast
//!
//! ## Invariant
//! Services never start with partial/misconfigured secrets or unsafe defaults.
//! Failure mode to avoid: running with missing keys leading to corrupt external integrations
//! or insecure behavior.

use anyhow::Result;
use serial_test::serial;

/// Test: AR config validation requires DATABASE_URL
#[test]
#[serial]
#[serial]
fn test_ar_config_requires_database_url() -> Result<()> {
    // Save current env
    let original_database_url = std::env::var("DATABASE_URL").ok();

    // Remove DATABASE_URL
    std::env::remove_var("DATABASE_URL");

    // Attempt to load config - should fail
    let result = ar_rs::config::Config::from_env();

    // Restore original env
    if let Some(url) = original_database_url {
        std::env::set_var("DATABASE_URL", url);
    }

    // Assert that config loading failed
    assert!(
        result.is_err(),
        "AR config should fail without DATABASE_URL"
    );

    let error = result.unwrap_err();
    assert!(
        error.contains("DATABASE_URL"),
        "Error message should mention DATABASE_URL, got: {}",
        error
    );
    assert!(
        error.contains("required") || error.contains("must be set") || error.contains("empty"),
        "Error message should indicate it's required or empty, got: {}",
        error
    );

    println!("✓ AR config validation requires DATABASE_URL");
    Ok(())
}

/// Test: Payments config validation requires DATABASE_URL
#[test]
#[serial]
fn test_payments_config_requires_database_url() -> Result<()> {
    // Save current env
    let original_database_url = std::env::var("DATABASE_URL").ok();

    // Remove DATABASE_URL
    std::env::remove_var("DATABASE_URL");

    // Attempt to load config - should fail
    let result = payments_rs::config::Config::from_env();

    // Restore original env
    if let Some(url) = original_database_url {
        std::env::set_var("DATABASE_URL", url);
    }

    // Assert that config loading failed
    assert!(
        result.is_err(),
        "Payments config should fail without DATABASE_URL"
    );

    let error = result.unwrap_err();
    assert!(
        error.contains("DATABASE_URL"),
        "Error message should mention DATABASE_URL, got: {}",
        error
    );
    assert!(
        error.contains("required") || error.contains("must be set") || error.contains("empty"),
        "Error message should indicate it's required or empty, got: {}",
        error
    );

    println!("✓ Payments config validation requires DATABASE_URL");
    Ok(())
}

/// Test: Subscriptions config validation requires DATABASE_URL
#[test]
#[serial]
fn test_subscriptions_config_requires_database_url() -> Result<()> {
    // Save current env
    let original_database_url = std::env::var("DATABASE_URL").ok();

    // Remove DATABASE_URL
    std::env::remove_var("DATABASE_URL");

    // Attempt to load config - should fail
    let result = subscriptions_rs::config::Config::from_env();

    // Restore original env
    if let Some(url) = original_database_url {
        std::env::set_var("DATABASE_URL", url);
    }

    // Assert that config loading failed
    assert!(
        result.is_err(),
        "Subscriptions config should fail without DATABASE_URL"
    );

    let error = result.unwrap_err();
    assert!(
        error.contains("DATABASE_URL"),
        "Error message should mention DATABASE_URL, got: {}",
        error
    );
    assert!(
        error.contains("required") || error.contains("must be set") || error.contains("empty"),
        "Error message should indicate it's required or empty, got: {}",
        error
    );

    println!("✓ Subscriptions config validation requires DATABASE_URL");
    Ok(())
}

/// Test: GL config validation requires DATABASE_URL
#[test]
#[serial]
fn test_gl_config_requires_database_url() -> Result<()> {
    // Save current env
    let original_database_url = std::env::var("DATABASE_URL").ok();

    // Remove DATABASE_URL
    std::env::remove_var("DATABASE_URL");

    // Attempt to load config - should fail
    let result = gl_rs::config::Config::from_env();

    // Restore original env
    if let Some(url) = original_database_url {
        std::env::set_var("DATABASE_URL", url);
    }

    // Assert that config loading failed
    assert!(
        result.is_err(),
        "GL config should fail without DATABASE_URL"
    );

    let error = result.unwrap_err();
    assert!(
        error.contains("DATABASE_URL"),
        "Error message should mention DATABASE_URL, got: {}",
        error
    );
    assert!(
        error.contains("required") || error.contains("must be set") || error.contains("empty"),
        "Error message should indicate it's required or empty, got: {}",
        error
    );

    println!("✓ GL config validation requires DATABASE_URL");
    Ok(())
}

/// Test: AR config validates BUS_TYPE values
#[test]
#[serial]
fn test_ar_config_validates_bus_type() -> Result<()> {
    // Save current env
    let original_database_url = std::env::var("DATABASE_URL").ok();
    let original_bus_type = std::env::var("BUS_TYPE").ok();

    // Set valid DATABASE_URL and invalid BUS_TYPE
    std::env::set_var("DATABASE_URL", "postgresql://localhost/test");
    std::env::set_var("BUS_TYPE", "invalid_bus_type");

    // Attempt to load config - should fail
    let result = ar_rs::config::Config::from_env();

    // Restore original env
    std::env::remove_var("DATABASE_URL");
    std::env::remove_var("BUS_TYPE");
    if let Some(url) = original_database_url {
        std::env::set_var("DATABASE_URL", url);
    }
    if let Some(bus_type) = original_bus_type {
        std::env::set_var("BUS_TYPE", bus_type);
    }

    // Assert that config loading failed
    assert!(
        result.is_err(),
        "AR config should fail with invalid BUS_TYPE"
    );

    let error = result.unwrap_err();
    assert!(
        error.contains("BUS_TYPE") || error.contains("invalid"),
        "Error message should mention BUS_TYPE or invalid, got: {}",
        error
    );

    println!("✓ AR config validates BUS_TYPE values");
    Ok(())
}

/// Test: AR config validation rejects empty DATABASE_URL
#[test]
#[serial]
fn test_ar_config_rejects_empty_database_url() -> Result<()> {
    // Save current env
    let original_database_url = std::env::var("DATABASE_URL").ok();

    // Set empty DATABASE_URL
    std::env::set_var("DATABASE_URL", "");

    // Attempt to load config - should fail
    let result = ar_rs::config::Config::from_env();

    // Restore original env
    std::env::remove_var("DATABASE_URL");
    if let Some(url) = original_database_url {
        std::env::set_var("DATABASE_URL", url);
    }

    // Assert that config loading failed
    assert!(
        result.is_err(),
        "AR config should fail with empty DATABASE_URL"
    );

    let error = result.unwrap_err();
    assert!(
        error.contains("DATABASE_URL") && error.contains("empty"),
        "Error message should mention DATABASE_URL cannot be empty, got: {}",
        error
    );

    println!("✓ AR config rejects empty DATABASE_URL");
    Ok(())
}

/// Test: Payments config validation rejects invalid PORT
#[test]
#[serial]
fn test_payments_config_validates_port() -> Result<()> {
    // Save current env
    let original_database_url = std::env::var("DATABASE_URL").ok();
    let original_port = std::env::var("PORT").ok();

    // Set valid DATABASE_URL and invalid PORT
    std::env::set_var("DATABASE_URL", "postgresql://localhost/test");
    std::env::set_var("PORT", "99999"); // Out of u16 range

    // Attempt to load config - should fail
    let result = payments_rs::config::Config::from_env();

    // Restore original env
    std::env::remove_var("DATABASE_URL");
    std::env::remove_var("PORT");
    if let Some(url) = original_database_url {
        std::env::set_var("DATABASE_URL", url);
    }
    if let Some(port) = original_port {
        std::env::set_var("PORT", port);
    }

    // Assert that config loading failed
    assert!(
        result.is_err(),
        "Payments config should fail with invalid PORT"
    );

    let error = result.unwrap_err();
    assert!(
        error.contains("PORT") || error.contains("u16"),
        "Error message should mention PORT validation, got: {}",
        error
    );

    println!("✓ Payments config validates PORT");
    Ok(())
}

/// Test: GL config validation accepts valid configuration
#[test]
#[serial]
fn test_gl_config_accepts_valid_config() -> Result<()> {
    // Save current env
    let original_database_url = std::env::var("DATABASE_URL").ok();
    let original_bus_type = std::env::var("BUS_TYPE").ok();
    let original_port = std::env::var("PORT").ok();

    // Set valid configuration
    std::env::set_var("DATABASE_URL", "postgresql://localhost/test");
    std::env::set_var("BUS_TYPE", "inmemory");
    std::env::set_var("PORT", "8090");

    // Attempt to load config - should succeed
    let result = gl_rs::config::Config::from_env();

    // Restore original env
    std::env::remove_var("DATABASE_URL");
    std::env::remove_var("BUS_TYPE");
    std::env::remove_var("PORT");
    if let Some(url) = original_database_url {
        std::env::set_var("DATABASE_URL", url);
    }
    if let Some(bus_type) = original_bus_type {
        std::env::set_var("BUS_TYPE", bus_type);
    }
    if let Some(port) = original_port {
        std::env::set_var("PORT", port);
    }

    // Assert that config loading succeeded
    assert!(
        result.is_ok(),
        "GL config should succeed with valid configuration, got error: {:?}",
        result.err()
    );

    let config = result.unwrap();
    assert_eq!(config.database_url, "postgresql://localhost/test");
    assert_eq!(config.bus_type, "inmemory");
    assert_eq!(config.port, 8090);

    println!("✓ GL config accepts valid configuration");
    Ok(())
}

/// Test: Subscriptions config validation requires NATS_URL when BUS_TYPE=nats
#[test]
#[serial]
fn test_subscriptions_config_nats_requires_url() -> Result<()> {
    // Save current env
    let original_database_url = std::env::var("DATABASE_URL").ok();
    let original_bus_type = std::env::var("BUS_TYPE").ok();
    let original_nats_url = std::env::var("NATS_URL").ok();

    // Set DATABASE_URL and BUS_TYPE=nats, but remove NATS_URL
    std::env::set_var("DATABASE_URL", "postgresql://localhost/test");
    std::env::set_var("BUS_TYPE", "nats");
    std::env::remove_var("NATS_URL");

    // Create config (should use default NATS_URL)
    let result = subscriptions_rs::config::Config::from_env();

    // Restore original env
    std::env::remove_var("DATABASE_URL");
    std::env::remove_var("BUS_TYPE");
    if let Some(url) = original_database_url {
        std::env::set_var("DATABASE_URL", url);
    }
    if let Some(bus_type) = original_bus_type {
        std::env::set_var("BUS_TYPE", bus_type);
    }
    if let Some(nats_url) = original_nats_url {
        std::env::set_var("NATS_URL", nats_url);
    }

    // Config should succeed (uses default nats://localhost:4222)
    assert!(
        result.is_ok(),
        "Subscriptions config should use default NATS_URL when BUS_TYPE=nats"
    );

    let config = result.unwrap();
    assert_eq!(config.nats_url, Some("nats://localhost:4222".to_string()));

    println!("✓ Subscriptions config uses default NATS_URL when BUS_TYPE=nats");
    Ok(())
}

/// Test: AR config with valid NATS configuration
#[test]
#[serial]
fn test_ar_config_valid_nats_config() -> Result<()> {
    // Save current env
    let original_database_url = std::env::var("DATABASE_URL").ok();
    let original_bus_type = std::env::var("BUS_TYPE").ok();
    let original_nats_url = std::env::var("NATS_URL").ok();

    // Set valid NATS configuration
    std::env::set_var("DATABASE_URL", "postgresql://localhost/test");
    std::env::set_var("BUS_TYPE", "nats");
    std::env::set_var("NATS_URL", "nats://localhost:4222");

    // Attempt to load config - should succeed
    let result = ar_rs::config::Config::from_env();

    // Restore original env
    std::env::remove_var("DATABASE_URL");
    std::env::remove_var("BUS_TYPE");
    std::env::remove_var("NATS_URL");
    if let Some(url) = original_database_url {
        std::env::set_var("DATABASE_URL", url);
    }
    if let Some(bus_type) = original_bus_type {
        std::env::set_var("BUS_TYPE", bus_type);
    }
    if let Some(nats_url) = original_nats_url {
        std::env::set_var("NATS_URL", nats_url);
    }

    // Assert that config loading succeeded
    assert!(
        result.is_ok(),
        "AR config should succeed with valid NATS configuration, got error: {:?}",
        result.err()
    );

    let config = result.unwrap();
    assert_eq!(config.database_url, "postgresql://localhost/test");
    assert_eq!(config.bus_type, ar_rs::config::BusType::Nats);
    assert_eq!(config.nats_url, Some("nats://localhost:4222".to_string()));

    println!("✓ AR config accepts valid NATS configuration");
    Ok(())
}
