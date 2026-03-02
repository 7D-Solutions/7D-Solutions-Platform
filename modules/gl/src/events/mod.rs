//! GL event definitions
//!
//! This module defines GL's outbound event contracts — events emitted by the GL
//! module for consumption by downstream services.
//!
//! ## Accrual Events (Phase 24b)
//! - `gl.accrual_created`  → `contracts::AccrualCreatedPayload`
//! - `gl.accrual_reversed` → `contracts::AccrualReversedPayload`
//!
//! ## FX Events (Phase 23a)
//! - `fx.rate_updated`           → `contracts::FxRateUpdatedPayload`
//! - `gl.fx_revaluation_posted`  → `contracts::FxRevaluationPostedPayload`
//! - `gl.fx_realized_posted`     → `contracts::FxRealizedPostedPayload`
//!
//! ## Supporting types
//! - `contracts::CashFlowClass` — operating/investing/financing/non_cash classification
//! - `contracts::CashFlowClassification` — account_ref → CashFlowClass mapping
//! - `contracts::ReversalPolicy` — when/how to auto-reverse an accrual

pub mod contracts;
pub mod envelope;

pub use contracts::{
    // Accrual events
    build_accrual_created_envelope,
    build_accrual_reversed_envelope,
    // FX events (Phase 23a)
    build_fx_rate_updated_envelope,
    build_fx_realized_posted_envelope,
    build_fx_revaluation_posted_envelope,
    AccrualCreatedPayload,
    AccrualReversedPayload,
    CashFlowClass,
    CashFlowClassification,
    FxRateUpdatedPayload,
    FxRealizedPostedPayload,
    FxRevaluationPostedPayload,
    ReversalPolicy,
    EVENT_TYPE_ACCRUAL_CREATED,
    EVENT_TYPE_ACCRUAL_REVERSED,
    EVENT_TYPE_FX_RATE_UPDATED,
    EVENT_TYPE_FX_REALIZED_POSTED,
    EVENT_TYPE_FX_REVALUATION_POSTED,
    GL_ACCRUAL_SCHEMA_VERSION,
    GL_FX_SCHEMA_VERSION,
    MUTATION_CLASS_DATA_MUTATION,
    MUTATION_CLASS_REVERSAL,
};
