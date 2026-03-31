# Party Client Migration Guide

## From hand-written fetch to `@7d/party-client`

### Before (hand-written fetch)

```typescript
// Example: creating a company with `fetch`
const response = await fetch("http://localhost:8098/api/party/companies", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({ display_name: "Acme Corp", legal_name: "Acme Corp LLC" }),
});
if (!response.ok) throw new Error(response.statusText);
const payload = await response.json();
```

### After (generated client)

```typescript
import { createPartyClient, CreateCompanyRequest } from "@7d/party-client";

const client = createPartyClient({
  baseUrl: "http://localhost:8098",
  token: jwt,
});

const { data: party } = await client.POST("/api/party/companies", {
  body: {
    display_name: "Acme Corp",
    legal_name: "Acme Corp LLC",
  } satisfies CreateCompanyRequest,
});
```

`@7d/party-client` exposes typed helpers for parties, contacts, and addresses (e.g., `CreateContactRequest`, `DataResponse_Contact`, `PrimaryContactEntry`). All requests are serialized through `openapi-fetch`, so headers and status codes are fully typed.

## Regenerating the client

1. From `modules/party`, produce the OpenAPI JSON without running the whole service:

   ```bash
   cd modules/party
   cargo run -p party-rs --bin openapi_dump > ../../clients/party/openapi.json
   ```

2. From `clients/party`, regenerate the TypeScript declarations:

   ```bash
   cd clients/party
   npx openapi-typescript openapi.json -o src/party.d.ts
   ```

3. (Optional) Run the consumer proof to exercise the generated client against a live Party service:

   ```bash
   npx tsx tests/consumer-proof.ts
   ```
