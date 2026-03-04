# Compose Pinned Image Deploy

`docker-compose.services.yml` remains the dev source-build file (`build:`). Production uses an override file with pinned `image:` tags.

## Files

- Base (dev): `docker-compose.services.yml`
- Production override: `docker-compose.production.yml`
- Release manifest: `deploy/production/compose-release-manifest.json`

## Production deploy (no build step)

```bash
docker compose -f docker-compose.services.yml -f docker-compose.production.yml up -d
```

This must run without `--build`.

## Optional env materialization

If you want explicit env assignments from the manifest:

```bash
./scripts/release/manifest_to_env.sh deploy/production/compose-release-manifest.json .env.release

docker compose --env-file .env.release \
  -f docker-compose.services.yml \
  -f docker-compose.production.yml up -d
```

## Verification

```bash
docker compose -f docker-compose.services.yml -f docker-compose.production.yml ps
NATS_URL="nats://platform:${NATS_AUTH_TOKEN:-dev-nats-token}@localhost:4222" ./scripts/proofs_runbook.sh
```

Expected: clean `7d-*` container names, HTTP services healthy, NATS checks pass, and runbook `33/33` crate pass.

## Rollback

1. Restore previous `deploy/production/compose-release-manifest.json`.
2. Re-tag images (if needed) to the previous tags from the manifest.
3. Re-run deploy command.

```bash
git checkout <previous-sha> -- deploy/production/compose-release-manifest.json
./scripts/release/manifest_to_env.sh deploy/production/compose-release-manifest.json .env.release
docker compose --env-file .env.release -f docker-compose.services.yml -f docker-compose.production.yml up -d
```
