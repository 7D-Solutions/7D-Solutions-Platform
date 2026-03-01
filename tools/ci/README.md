# ci

Repository CI/lint helper scripts for policy checks and contract validation.
Used in pipelines and locally before merging.

Run:
```bash
bash tools/ci/check-contract-versions.sh
bash tools/ci/lint-no-cross-module-imports.sh
```

Each script exits non-zero on failure and prints actionable diagnostics.
