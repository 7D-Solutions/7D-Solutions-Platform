# Changelog

## 0.1.0
- Initial Payments module scaffold
- Minimal Axum server with health endpoint
- Webhook signature verification ready (hmac, sha2)
- Event-driven integration ready (consumes ar.payment.collection.requested, emits payments.*)
