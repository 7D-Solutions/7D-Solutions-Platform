# Carrier Adapters

The shipping-receiving module supports three real carrier adapters and one stub adapter:

| Carrier | Code | Use case |
|---------|------|----------|
| FedEx | `fedex` | Production rate quotes, labels, and tracking |
| UPS | `ups` | Production rate quotes, labels, and tracking |
| USPS | `usps` | Production rate quotes, labels, and tracking |
| Stub | `stub` | Local development and tests only |

## How To Use A Real Adapter

1. Create or update the carrier request with `carrier_code` set to `fedex`, `ups`, or `usps`.
2. Configure the matching credentials in the Integrations module.
3. Run a live request end to end and confirm the carrier request transitions from `pending` to `submitted` and then `completed`.

## Stub Adapter

`stub` returns canned responses for tests and local runs. It should not be used for staging or production traffic.

When the module starts outside `dev` or `development`, it logs a warning so stub usage is easy to catch before a deployment goes live.

## Practical Checks

- `fedex`, `ups`, and `usps` should resolve to real sandbox or production credentials before you ship.
- `stub` should only appear in fixtures, tests, and local developer flows.
- If you see the startup warning in a shared environment, switch the request flow away from `stub` before you promote the deployment.
