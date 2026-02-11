# Products Composition Standard

Products contain:
- No domain logic
- No direct DB
- No cross-module calls bypassing contracts

Products wire modules via:
- OpenAPI
- Event bus subscriptions
- Configuration

Example:
TrashTech = Subscriptions + AR + Payments + Notifications
