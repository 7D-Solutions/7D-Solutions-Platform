# Operations Standard

## Logging
- Structured logs (JSON)
- Correlation ID required

## Metrics
- request_count
- error_rate
- event_processing_latency

## Alerts
- Failed event processing > threshold
- Payment retry max reached
- GL posting rejection spike

## Deployment
- Each module independently deployable
- No shared DB
