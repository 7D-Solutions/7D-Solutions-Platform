# Accounts Receivable API

The authoritative API contract for this module is maintained at:

**[/contracts/ar/ar-v1.yaml](/contracts/ar/ar-v1.yaml)**

## Contract Version

Current contract version: **v0.1.0**

## Endpoints

See the OpenAPI specification in `/contracts/ar/` for:
- Customer management
- Subscription management
- Invoice operations
- Charge processing
- Refund handling
- Dispute management
- Payment method management
- Webhook integration
- Event access

## Payment Method Handling

This module stores only payment method **references** and non-sensitive metadata.

No raw card data, API keys, or gateway credentials are stored.

Sensitive payment processing is delegated to external payment processors.

## Implementation

The Rust implementation in `../src/` implements this contract.

Any deviation from the contract is a bug and must be corrected.
