# Integrations GCP Secrets Key Runbook

**Added: bd-zpi79.1**

## Purpose

The integrations service uses a 32-byte AES-256-GCM key (`INTEGRATIONS_SECRETS_KEY`) to
encrypt per-tenant QBO webhook verifier tokens and carrier credentials at rest. This
runbook covers provisioning the key in Google Secret Manager, granting the service account
access, configuring the production environment, and rotating the key.

---

## How the key is loaded at startup

The service tries two sources in order:

1. **Google Secret Manager** — used when both `GOOGLE_APPLICATION_CREDENTIALS` and
   `GCP_PROJECT_ID` are set. The secret name defaults to `integrations-secrets-key`
   and can be overridden with `GCP_SECRET_NAME`. Any fetch failure logs a warning and
   falls through to source 2.

2. **`INTEGRATIONS_SECRETS_KEY` env var** — accepted as 64 hex characters or 44-char
   base64 (with padding). The service panics at startup if this fallback is also absent
   or decodes to something other than 32 bytes.

In production, source 1 (GCP) is the authoritative path. Source 2 serves local dev and
disaster recovery.

---

## Step 1 — Generate the 32-byte key

```bash
# Generate a cryptographically random 32-byte key and print as hex
openssl rand -hex 32
# Example output: a3f1e8c7b2d094...  (64 hex chars)

# Or as base64 if you prefer that format
openssl rand -base64 32
# Example output: 5Rz4+JKm...=  (44 chars with padding)
```

Store the raw output — you will use it as the secret payload in GCP.

---

## Step 2 — Create the secret in GCP Secret Manager

```bash
# Set your project ID
export GCP_PROJECT=your-project-id

# Create the secret (first time only)
gcloud secrets create integrations-secrets-key \
  --project="${GCP_PROJECT}" \
  --replication-policy=automatic

# Add the key value as version 1
echo -n "PASTE_64_HEX_CHARS_HERE" | \
  gcloud secrets versions add integrations-secrets-key \
    --project="${GCP_PROJECT}" \
    --data-file=-
```

Verify the secret version is `ENABLED`:

```bash
gcloud secrets versions list integrations-secrets-key --project="${GCP_PROJECT}"
```

---

## Step 3 — Grant the service account access

The integrations container runs as a GCP service account. Grant it
`secretmanager.secretAccessor` on the specific secret (not project-wide):

```bash
# Replace with your actual service account email
export SA_EMAIL=integrations@your-project-id.iam.gserviceaccount.com

gcloud secrets add-iam-policy-binding integrations-secrets-key \
  --project="${GCP_PROJECT}" \
  --member="serviceAccount:${SA_EMAIL}" \
  --role="roles/secretmanager.secretAccessor"
```

Confirm the binding:

```bash
gcloud secrets get-iam-policy integrations-secrets-key --project="${GCP_PROJECT}"
```

---

## Step 4 — Configure the production environment

Add these three variables to the integrations service in your docker-compose or
secrets manager overlay:

| Variable | Value | Notes |
|----------|-------|-------|
| `GOOGLE_APPLICATION_CREDENTIALS` | `/run/secrets/integrations-sa.json` | Path to the mounted service account JSON key |
| `GCP_PROJECT_ID` | `your-project-id` | GCP project that hosts the secret |
| `GCP_SECRET_NAME` | `integrations-secrets-key` | Only needed if you used a non-default name |

Do **not** set `INTEGRATIONS_SECRETS_KEY` in production — the env var fallback is for
local dev and disaster recovery only. Mixing both sources in production will work
(GCP wins), but it adds confusion during incident response.

Mount the service account JSON key as a Docker secret:

```yaml
# docker-compose.prod.yml excerpt
services:
  integrations:
    environment:
      GOOGLE_APPLICATION_CREDENTIALS: /run/secrets/integrations-sa.json
      GCP_PROJECT_ID: your-project-id
    secrets:
      - integrations-sa
secrets:
  integrations-sa:
    file: ./secrets/integrations-sa.json
```

After updating the compose file, restart the integrations container and confirm startup
logs show `GCP` (not `falling back to env var`):

```bash
docker compose logs integrations | grep -i 'gcp\|secrets_key\|fallback'
```

---

## Key rotation procedure

Rotating the key requires re-encrypting all existing rows because ciphertext produced
with the old key cannot be decrypted with the new key.

### 1 — Generate a new 32-byte key

```bash
openssl rand -hex 32
# Save this as NEW_KEY
```

### 2 — Add a new secret version in GCP (do not delete the old version yet)

```bash
echo -n "${NEW_KEY}" | \
  gcloud secrets versions add integrations-secrets-key \
    --project="${GCP_PROJECT}" \
    --data-file=-
```

### 3 — Re-encrypt existing rows

The integrations service does not yet have a built-in re-encryption migration.
Run a one-off script that:

1. Reads every row from `integrations_qbo_webhook_secrets` and
   `integrations_carrier_credentials` using the old key.
2. Re-encrypts each row with the new key.
3. Writes back (upsert).

Keep the old key available in a temporary env var while the re-encryption script runs.
Do this during a low-traffic window.

### 4 — Promote the new version as the active key

Update the `INTEGRATIONS_SECRETS_KEY` secret in GCP to point to the new version
(or rely on the `latest` alias if you added a new version in step 2).

Restart the integrations container:

```bash
AGENTCORE_WATCHER_OVERRIDE=1 docker restart integrations
```

Confirm clean startup — no `fallback` warnings in logs.

### 5 — Disable the old secret version

Once the service is running cleanly with the new key and all rows are re-encrypted:

```bash
# List versions to find the old version number (e.g. 1)
gcloud secrets versions list integrations-secrets-key --project="${GCP_PROJECT}"

# Disable (not destroy) the old version first to allow rollback
gcloud secrets versions disable 1 \
  --secret=integrations-secrets-key \
  --project="${GCP_PROJECT}"
```

Wait 24 hours; if no incidents, destroy:

```bash
gcloud secrets versions destroy 1 \
  --secret=integrations-secrets-key \
  --project="${GCP_PROJECT}"
```

---

## Rollback

If the new key causes startup failures, set `INTEGRATIONS_SECRETS_KEY` to the old hex
value in the environment as an emergency fallback, then restart. This bypasses GCP and
gives you time to diagnose.

---

## Verification checklist

- [ ] Secret exists: `gcloud secrets describe integrations-secrets-key --project=…`
- [ ] IAM binding present: `gcloud secrets get-iam-policy integrations-secrets-key --project=…`
- [ ] Env vars set in prod compose: `GOOGLE_APPLICATION_CREDENTIALS`, `GCP_PROJECT_ID`
- [ ] Service account JSON mounted and readable inside container
- [ ] Startup logs show GCP path used (no `fallback to env var` warning)
- [ ] QBO webhook round-trip passes after deploy
