//! FX revaluation helper functions.
//!
//! Pure and near-pure utilities used by the revaluation orchestration:
//! - Deterministic event ID generation (idempotency)
//! - Transactional FX rate lookup
//! - Signed balance conversion

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::services::currency_conversion::{self, ConversionError, RateSnapshot};

/// UUID namespace for deterministic revaluation event IDs.
/// Generated once and frozen — changing this breaks idempotency.
const REVAL_NAMESPACE: Uuid = Uuid::from_bytes([
    0x8a, 0x3b, 0x4c, 0x5d, 0x6e, 0x7f, 0x48, 0x90, 0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x07, 0x18,
]);

/// Generate a deterministic UUID v5 for the revaluation event.
///
/// This ensures that the same period always produces the same event ID,
/// providing idempotency via the UNIQUE constraint on source_event_id.
pub(crate) fn deterministic_event_id(period_id: Uuid) -> Uuid {
    Uuid::new_v5(&REVAL_NAMESPACE, period_id.as_bytes())
}

/// Look up the latest FX rate within a transaction.
///
/// Tries both directions: base/quote and quote/base.
pub(crate) async fn lookup_rate_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    foreign_currency: &str,
    reporting_currency: &str,
    as_of: DateTime<Utc>,
) -> Result<Option<RateSnapshot>, sqlx::Error> {
    // Try direct: foreign_currency as base, reporting_currency as quote
    let row = sqlx::query(
        r#"
        SELECT id, rate, inverse_rate, effective_at, base_currency, quote_currency
        FROM fx_rates
        WHERE tenant_id = $1
          AND (
            (base_currency = $2 AND quote_currency = $3)
            OR (base_currency = $3 AND quote_currency = $2)
          )
          AND effective_at <= $4
        ORDER BY effective_at DESC, created_at ASC, id ASC
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(foreign_currency)
    .bind(reporting_currency)
    .bind(as_of)
    .fetch_optional(&mut **tx)
    .await?;

    Ok(row.map(|r| RateSnapshot {
        rate_id: r.get("id"),
        rate: r.get("rate"),
        inverse_rate: r.get("inverse_rate"),
        effective_at: r.get("effective_at"),
        base_currency: r.get("base_currency"),
        quote_currency: r.get("quote_currency"),
    }))
}

/// Convert a signed balance amount using the appropriate rate direction.
///
/// `convert_amount` rejects negative inputs, so we convert the absolute value
/// and re-apply the sign.
pub(crate) fn convert_with_sign(
    balance_minor: i64,
    rate: &RateSnapshot,
    from: &str,
    to: &str,
) -> Result<i64, ConversionError> {
    if balance_minor == 0 {
        return Ok(0);
    }

    let abs_balance = balance_minor.unsigned_abs() as i64;
    let converted = currency_conversion::convert_amount(abs_balance, rate, from, to)?;

    if balance_minor < 0 {
        Ok(-converted.reporting_amount_minor)
    } else {
        Ok(converted.reporting_amount_minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_event_id_is_stable() {
        let period_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let id1 = deterministic_event_id(period_id);
        let id2 = deterministic_event_id(period_id);
        assert_eq!(id1, id2, "Same period must produce same event ID");
    }

    #[test]
    fn different_periods_produce_different_ids() {
        let p1 = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let p2 = Uuid::parse_str("660e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_ne!(
            deterministic_event_id(p1),
            deterministic_event_id(p2),
            "Different periods must produce different event IDs"
        );
    }

    #[test]
    fn convert_with_sign_positive() {
        let rate = RateSnapshot {
            rate_id: Uuid::new_v4(),
            rate: 1.085,
            inverse_rate: 1.0 / 1.085,
            effective_at: Utc::now(),
            base_currency: "EUR".to_string(),
            quote_currency: "USD".to_string(),
        };
        let result = convert_with_sign(100000, &rate, "EUR", "USD").unwrap();
        assert_eq!(result, 108500);
    }

    #[test]
    fn convert_with_sign_negative() {
        let rate = RateSnapshot {
            rate_id: Uuid::new_v4(),
            rate: 1.085,
            inverse_rate: 1.0 / 1.085,
            effective_at: Utc::now(),
            base_currency: "EUR".to_string(),
            quote_currency: "USD".to_string(),
        };
        let result = convert_with_sign(-100000, &rate, "EUR", "USD").unwrap();
        assert_eq!(result, -108500);
    }

    #[test]
    fn convert_with_sign_zero() {
        let rate = RateSnapshot {
            rate_id: Uuid::new_v4(),
            rate: 1.085,
            inverse_rate: 1.0 / 1.085,
            effective_at: Utc::now(),
            base_currency: "EUR".to_string(),
            quote_currency: "USD".to_string(),
        };
        let result = convert_with_sign(0, &rate, "EUR", "USD").unwrap();
        assert_eq!(result, 0);
    }
}
