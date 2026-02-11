# SLO — auth-rs v1.4.x

## SLO Targets (Production)
1) Availability (readiness):
- SLO: 99.9% monthly for /health/ready
- Error budget: 43m 49s / month

2) Login success latency:
- p95 < 500ms over 5 minutes
- p99 < 1200ms over 5 minutes

3) Refresh success latency:
- p95 < 400ms
- p99 < 900ms

4) Security correctness:
- Replay detection must alert at > 0
- Lockout must engage at configured threshold

## Measurement
- Availability: probe /health/ready via Prometheus blackbox or service monitor
- Latency: http_request_duration_seconds histogram
- Security: auth_refresh_replay_total, auth_login_total{reason="locked"}, auth_*_total{reason="hash_busy"}

## Error Budget Policy
If error budget burn > 50% mid-month:
- Freeze feature work
- Only reliability/security improvements until burn returns to normal
