//! Revenue Recognition (Revrec) module for GL (Phase 24a)
//!
//! This module defines the data model and event contracts for ASC 606 / IFRS 15
//! revenue recognition within the GL module.
//!
//! ## Events emitted
//! - `revrec.contract_created`   — new revenue contract with obligations locked
//! - `revrec.schedule_created`   — recognition amortization schedule computed
//! - `revrec.recognition_posted` — revenue recognized for a period (DR deferred / CR revenue)
//! - `revrec.contract_modified`  — contract amended (price change, obligations updated)
//!
//! ## Data model
//! - `ContractCreatedPayload`   — root entity with performance obligations embedded
//! - `PerformanceObligation`    — distinct promise, carries allocation + recognition pattern
//! - `RecognitionPattern`       — ratable-over-time | point-in-time | usage-based
//! - `ScheduleCreatedPayload`   — amortization table (Vec<ScheduleLine>)
//! - `ScheduleLine`             — one period entry (period, amount, accounts)
//! - `RecognitionPostedPayload` — single period recognition run
//! - `ContractModifiedPayload`  — amendment with allocation changes
//! - `AllocationChange`         — before/after amounts per obligation
//! - `ModificationType`         — price_change | term_extension | obligation_added | etc.

pub mod contracts;
pub mod schedule_builder;

pub use schedule_builder::{generate_schedule, ScheduleBuildError};

pub use contracts::{
    build_contract_created_envelope,
    build_contract_modified_envelope,
    build_recognition_posted_envelope,
    build_schedule_created_envelope,
    AllocationChange,
    ContractCreatedPayload,
    ContractModifiedPayload,
    ModificationType,
    PerformanceObligation,
    RecognitionPattern,
    RecognitionPostedPayload,
    ScheduleCreatedPayload,
    ScheduleLine,
    EVENT_TYPE_CONTRACT_CREATED,
    EVENT_TYPE_CONTRACT_MODIFIED,
    EVENT_TYPE_RECOGNITION_POSTED,
    EVENT_TYPE_SCHEDULE_CREATED,
    MUTATION_CLASS_DATA_MUTATION,
    REVREC_SCHEMA_VERSION,
};
