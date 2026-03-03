# ADR-017: Blob Storage Contract for Document Artifacts

**Date:** 2026-03-03
**Status:** Accepted
**Deciders:** Platform Orchestrator, Engineering Team
**Technical Story:** bd-32uk4 — Blob storage strategy for doc-mgmt + customer portal

## Context and Problem Statement

`doc-mgmt` and `customer-portal` both need durable blob storage for uploaded and rendered document artifacts. If each service defines its own bucket layout, auth boundary, or retention behavior, tenant isolation and retrieval consistency will fail over time.

This ADR defines one storage contract both services must implement.

## Decision Outcome

Use an S3-compatible object store as the canonical backend in all environments, with strict tenant-scoped keys, SSE encryption at rest, and a controlled upload/download boundary.

## 1. Supported Backends and Environments

### Canonical backend

- Production/staging: S3-compatible object storage (AWS S3, Cloudflare R2, MinIO gateway, or equivalent S3 API).
- Local/dev: MinIO (S3-compatible) via local Docker stack.

### Required environment variables

- `BLOB_PROVIDER=s3`
- `BLOB_REGION=<region>`
- `BLOB_ENDPOINT=<optional custom endpoint; required for non-AWS S3 providers>`
- `BLOB_BUCKET_DOCS=<shared bucket or service-specific bucket name>`
- `BLOB_ACCESS_KEY_ID=<access key>`
- `BLOB_SECRET_ACCESS_KEY=<secret key>`
- `BLOB_PRESIGN_TTL_SECONDS=<default 900, max 3600>`
- `BLOB_MAX_UPLOAD_BYTES=<default 26214400 (25 MiB)>`

### Credential strategy

- Production/staging prefer workload identity + short-lived credentials (IAM role, OIDC federation, or equivalent) over static secrets.
- Static keys are allowed only for local/dev and emergency break-glass operation.
- Secrets must come from environment/secret manager, never from committed files.

## 2. Stable Storage Contract

### Object key scheme (required)

All blob keys must use:

`tenants/{tenant_id}/{service}/{artifact_type}/{entity_id}/{yyyy}/{mm}/{dd}/{object_id}-{safe_filename}`

Rules:
- `tenant_id` is required and canonical from `VerifiedClaims`.
- `service` is `doc-mgmt` or `customer-portal`.
- `artifact_type` examples: `upload`, `rendered`, `ack`, `attachment`.
- `object_id` is UUIDv7 (preferred) or UUIDv4.
- `safe_filename` must be normalized to lowercase ASCII + `[-._a-z0-9]` only.

### Tenancy and encryption invariants

- Cross-tenant key reuse is forbidden.
- All reads/writes must validate requested tenant matches JWT tenant.
- Server-side encryption at rest is mandatory (`SSE-S3` minimum; `SSE-KMS` preferred in production).

### Retention/lifecycle hooks

- Object metadata must include:
  - `x-platform-tenant-id`
  - `x-platform-retention-class`
  - `x-platform-legal-hold` (`true|false`)
  - `x-platform-source-service`
- Services must publish lifecycle intents (retain/delete/hold/release) through domain events and apply asynchronous object lifecycle jobs.
- Physical delete is blocked while legal hold is `true`.

## 3. Upload/Download Security Model

### Upload path

- Default: proxy-through-service for create authorization + audit logging, then service may issue short-lived presigned PUT.
- Presigned uploads are single-object only and must pin:
  - bucket
  - exact key
  - content-length upper bound (`BLOB_MAX_UPLOAD_BYTES`)
  - content-type allowlist
- Direct client bucket credentials are forbidden.

### Download path

- Default: service verifies entitlement and returns short-lived presigned GET (15 minutes default).
- Highly sensitive artifacts may require proxy download when policy requires inline redaction/watermarking.

### Size limits and MIME allowlist

- Default max upload size: 25 MiB (override only by explicit service config).
- Allowed MIME types (initial set):
  - `application/pdf`
  - `image/png`
  - `image/jpeg`
  - `text/plain`
  - `application/vnd.openxmlformats-officedocument.wordprocessingml.document`
- Block `application/x-msdownload`, `application/javascript`, and unspecified binary MIME types unless explicitly approved.

### Audit logging requirements

Each upload/download/delete/hold/release must emit an audit event with:
- `tenant_id`
- `actor_id` and `actor_type`
- `service`
- `bucket`
- `object_key`
- `operation`
- `result` (`allowed`/`denied`/`failed`)
- `trace_id`
- `timestamp`

No blob operation may bypass audit logging.

## 4. Operational Guardrails

- Enable object versioning on production buckets.
- Deny public ACLs and public bucket policies.
- Enforce TLS for all object API requests.
- Add bucket lifecycle policy defaults only for objects without legal hold.

## 5. Downstream Beads Required to Follow This Contract

- `bd-1nvtn` — Customer portal service scaffold + external auth foundation
- `bd-3fx3b` — Customer portal: document access + status views + acknowledgments
- `bd-3i1aa` — Phase 58 Gate B: doc-mgmt perf+contracts+ops readiness

`doc-mgmt` ongoing work must align to this contract as well.

