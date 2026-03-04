# Compose Pinned Image Deploy

Production uses an overlay (`docker-compose.production.yml`) on top of the base
services file. The overlay pins every service to an immutable image tag and
configures Docker secrets. `docker-compose.services.yml` is never modified — it
stays as the source-build dev file.

## Source of truth

- Release manifest: `deploy/production/compose-release-manifest.json`
- Image overlay: `docker-compose.production.yml` (image: + secrets)
- Env var file: `.env.release` (generated from manifest)

## Generate .env.release

```bash
./scripts/release/manifest_to_env.sh deploy/production/compose-release-manifest.json .env.release
```

## Deploy (production)

```bash
# Generate env vars from manifest
./scripts/release/manifest_to_env.sh

# Start data layer
docker compose -f docker-compose.data.yml \
  -f docker-compose.production-data.yml up -d

# Start services with pinned images + secrets overlay
docker compose --env-file .env.release \
  -f docker-compose.services.yml \
  -f docker-compose.production.yml up -d
```

When both `build:` (from base) and `image:` (from overlay) are present, Compose
uses the existing image if it is available locally or in the registry, and skips
building. Never pass `--build` in production.

## Verify

```bash
docker compose --env-file .env.release \
  -f docker-compose.services.yml \
  -f docker-compose.production.yml ps
```

All 22 containers should be running with `7d-` prefixed names.

## Dev (build from source)

No overlay needed — just use the base services file:

```bash
docker compose -f docker-compose.data.yml up -d
docker compose -f docker-compose.services.yml up -d --build
```

## Rollback

1. Check out the previous manifest version.
2. Regenerate `.env.release`.
3. Re-deploy.

```bash
git checkout <previous-sha> -- deploy/production/compose-release-manifest.json
./scripts/release/manifest_to_env.sh
docker compose --env-file .env.release \
  -f docker-compose.services.yml \
  -f docker-compose.production.yml up -d
```

The previous images must still exist locally or in the registry. For safety,
keep at least 3 previous release tags.

## Cutting a new release

1. Build all images from source:
   ```bash
   docker compose -f docker-compose.services.yml build
   ```
2. Tag with release identifier:
   ```bash
   GIT_SHA=$(git rev-parse --short HEAD)
   TAG="release-${GIT_SHA}"
   # For each Rust service:
   docker tag 7d-services-ar:latest ghcr.io/7d-solutions/ar:${TAG}
   # ... repeat for all services
   ```
3. Update `deploy/production/compose-release-manifest.json` with new tags and SHA.
4. Regenerate `.env.release`:
   ```bash
   ./scripts/release/manifest_to_env.sh
   ```
5. Commit manifest + env file together.
