//! GL event definitions (Phase 24b)
//!
//! This module defines GL's outbound event contracts — events emitted by the GL
//! module for consumption by downstream services.
//!
//! ## Events
//! - `gl.accrual_created`  → `contracts::AccrualCreatedPayload`
//! - `gl.accrual_reversed` → `contracts::AccrualReversedPayload`
//!
//! ## Supporting types
//! - `contracts::CashFlowClass` — operating/investing/financing/non_cash classification
//! - `contracts::CashFlowClassification` — account_ref → CashFlowClass mapping
//! - `contracts::ReversalPolicy` — when/how to auto-reverse an accrual

pub mod contracts;
pub mod envelope;

pub use contracts::{
    build_accrual_created_envelope,
    build_accrual_reversed_envelope,
    AccrualCreatedPayload,
    AccrualReversedPayload,
    CashFlowClass,
    CashFlowClassification,
    ReversalPolicy,
    EVENT_TYPE_ACCRUAL_CREATED,
    EVENT_TYPE_ACCRUAL_REVERSED,
    GL_ACCRUAL_SCHEMA_VERSION,
    MUTATION_CLASS_DATA_MUTATION,
    MUTATION_CLASS_REVERSAL,
};
