# CI Proof Runbook

The CI proof runbook runs automatically on every merge to `main` and nightly at 3 AM UTC. It enforces the same evidence bar used to approve Phases 58-64: all crate tests pass, contract tests pass, and NATS is healthy.

## Retrieving Artifacts

1. Go to **Actions** → **Proof Runbook** in GitHub
2. Select a workflow run (green = all gates passed, red = at least one failed)
3. Scroll to the **Artifacts** section at the bottom of the run page
4. Download **proof-runbook-\<timestamp\>** (zip)

The zip contains:

```
proofs/<run_ts>/
  summary.txt                    – machine-readable gate results
  runbook.log                    – timestamped execution log
  cross-phase/
    platform-contracts.txt       – contract test output + exit code
  tests/
    modules/<crate>.txt          – per-crate test output (business modules)
    platform/<crate>.txt         – per-crate test output (platform modules)
  nats/
    nats_server_check.txt        – NATS /healthz + /varz
    nats_stream_ls.txt           – JetStream stream listing
    nats_consumers.txt           – JetStream consumer listing
```

## Interpreting `summary.txt`

Key fields:

| Field | Meaning |
|-------|---------|
| `crates_pass` / `crates_fail` / `crates_total` | Per-crate test results |
| `platform_contracts_exit_code` | `0` = contracts pass, non-zero = failure |
| `nats_exit_code` | `0` = NATS healthy, non-zero = failure |
| `contracts_pass` / `nats_pass` | Boolean convenience fields |

The pipeline **fails** if any of these are non-zero: `crates_fail`, `platform_contracts_exit_code`, or `nats_exit_code`.

## Running Locally

```bash
bash scripts/ci/proof-runbook-ci.sh
```

Requires Postgres and (optionally) NATS running locally. Set `DATABASE_URL` and `NATS_URL` environment variables. The script auto-discovers crates and produces the same artifact structure under `proofs/`.

## Triggers

| Trigger | When |
|---------|------|
| Push to `main` | Every merge |
| Nightly schedule | 3 AM UTC daily |
| Manual | Workflow dispatch from Actions tab |

## Relation to Local Proof Runbook

The local runbook (`scripts/proofs_runbook.sh`) is more comprehensive — it also captures service health endpoints, metrics, and outbox data from running Docker containers. The CI version covers the three core gates (crate tests, contracts, NATS) that are achievable without a full container fleet.
