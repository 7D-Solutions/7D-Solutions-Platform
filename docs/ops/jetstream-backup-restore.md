# JetStream Backup and Restore Runbook

## Purpose

This runbook defines how to inspect JetStream state, take a stream snapshot, restore it, and verify consumers resume correctly. It is scoped to non-production drills and staging/dev operations.

## Prerequisites

- Docker is running.
- NATS server is reachable:
  - client: `nats://localhost:4222`
  - monitoring: `http://localhost:8222`
- `jq` and `curl` are available on the host.
- `nats` CLI is available through `natsio/nats-box`.

## 1) Inspect Current JetStream State

List streams:

```bash
docker run --rm natsio/nats-box nats -s nats://host.docker.internal:4222 stream ls
```

List consumers for a stream:

```bash
docker run --rm natsio/nats-box nats -s nats://host.docker.internal:4222 consumer ls <STREAM>
```

Confirm server and JetStream health:

```bash
curl -fsS http://localhost:8222/healthz | jq .
curl -fsS http://localhost:8222/jsz | jq '{streams,consumers,messages,bytes}'
```

## 2) Backup / Snapshot a Stream

Take a backup including consumers:

```bash
mkdir -p /tmp/js-backup
docker run --rm -v /tmp/js-backup:/work natsio/nats-box \
  nats -s nats://host.docker.internal:4222 \
  stream backup <STREAM> /work --check --consumers --no-progress
```

Expected backup artifacts:

- `backup.json`
- `stream.tar.s2`

## 3) Restore a Stream

Restore from the backup directory:

```bash
docker run --rm -v /tmp/js-backup:/work natsio/nats-box \
  nats -s nats://host.docker.internal:4222 \
  stream restore /work --no-progress
```

## 4) Validate After Restore

Verify stream exists and has messages:

```bash
docker run --rm natsio/nats-box nats -s nats://host.docker.internal:4222 stream info <STREAM> --json \
  | jq '{name: .config.name, messages: .state.messages}'
```

Verify consumers are present:

```bash
docker run --rm natsio/nats-box nats -s nats://host.docker.internal:4222 consumer ls <STREAM> --names
```

Publish and consume a new message to prove resume:

```bash
docker run --rm natsio/nats-box nats -s nats://host.docker.internal:4222 publish <SUBJECT> "restore-check"
docker run --rm natsio/nats-box nats -s nats://host.docker.internal:4222 consumer next <STREAM> <CONSUMER> --count 1 --ack --raw
```

## 5) Automated Drill

Run the non-destructive drill script:

```bash
bash scripts/drills/jetstream_restore_drill.sh
```

The script:

- creates a unique drill stream and consumer
- publishes seed messages
- snapshots the stream (`stream backup`)
- deletes and restores it (`stream restore`)
- verifies consumer restoration and post-restore delivery
- prints `RESULT: PASS` or `RESULT: FAIL`

## Failure Modes and Response

- `backup artifacts missing`: backup command failed or wrong mount path.
  - Response: check `-v` volume mapping and target directory permissions.
- `consumer missing after restore`: restore completed without consumer metadata.
  - Response: ensure backup used `--consumers`; rerun drill.
- `message count decreased after restore`: incomplete restore or wrong backup source.
  - Response: repeat restore from known-good backup and compare `stream info` before/after.
- `monitoring endpoint unreachable`: NATS is up but monitor port is unavailable.
  - Response: verify `8222` is published and NATS config exposes monitoring.

## What Good Looks Like

- `stream backup` succeeds and produces `backup.json` + `stream.tar.s2`.
- `stream restore` completes without errors.
- Restored stream message count is equal to or greater than pre-backup count.
- Expected consumer is listed after restore.
- Consumer receives a newly published post-restore message.
