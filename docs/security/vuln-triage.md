# Vulnerability Triage Process

**Security scanning:** `.github/workflows/security.yml`
**Configuration:** `deny.toml` (workspace root)

## Security Owner

The Platform Orchestrator (BrightHill) is the security owner. All exceptions require
orchestrator approval before merging. Escalate unknown advisories to the user.

---

## How Scanning Works

| Tool | What it checks | When it runs |
|------|---------------|--------------|
| `cargo audit` | Known CVEs in `Cargo.lock` via RUSTSEC database | Every PR |
| `cargo deny` | Advisories + license allowlist + banned crates + source restrictions | Every PR + weekly |
| `gitleaks` (PR) | Secrets in commits introduced by the PR | Every PR |
| `gitleaks` (full) | Secrets across entire git history | Push to main + weekly + manual dispatch |

---

## Responding to a Failed Scan

### 1. cargo-audit / cargo-deny advisory failure

A `RUSTSEC-XXXX-YYYY` advisory was found in your dependency tree.

**Steps:**
1. Read the advisory: `cargo audit --explain RUSTSEC-XXXX-YYYY`
2. Determine: does this advisory affect our actual usage?
   - Does the vulnerable code path exist in our call graph?
   - Are we on a platform where the vulnerability is exploitable?
3. Check if a fixed version is available: `cargo update -p <crate>`
   - If yes: update the dependency and verify the build + tests pass.
   - If the fix is upstream but not yet released: see _Adding an Exception_ below.
4. Open a tracking issue: `[SEC] RUSTSEC-XXXX-YYYY — <crate> — <one-line risk>`
5. Assign P0 (exploitable in our call graph) or P1 (theoretical but not in our path).

**Resolution SLA:**
- P0 (exploitable): patch within 48 hours or disable the affected feature.
- P1 (theoretical): patch in the next sprint or add a justified exception.

### 2. cargo-deny license failure

An unknown or disallowed license was found.

**Steps:**
1. Identify the crate: `cargo deny check licenses 2>&1 | grep DENIED`
2. Check the actual license file in the crate source.
3. If the SPDX identifier is wrong (common with dual-licensed crates), add a
   `[[licenses.exceptions]]` entry in `deny.toml` with the correct SPDX.
4. If the license is a genuinely new type, get legal sign-off before allowing it.
   Copyleft (GPL/LGPL) licenses require explicit legal review — do not add without approval.

### 3. cargo-deny banned crate failure

A banned crate (`openssl`, etc.) was pulled in by a dependency.

**Steps:**
1. Find which dependency introduced it: `cargo tree -i <crate-name>`
2. Check if the dependency has a `rustls` feature flag: `cargo add <dep> --features rustls-tls`
3. If no alternative exists, open a discussion with the security owner before unblocking.

### 4. gitleaks secret detected

A high-confidence secret pattern was found in a commit.

**Steps:**
1. Identify the file and line: check the gitleaks scan output in the Actions log.
2. Determine if it is a real secret or a false positive.
   - **False positive** (test fixture, placeholder, example): add a `.gitleaks.toml`
     allowlist rule (see _Adding a gitleaks Exception_ below).
   - **Real secret**: proceed with _Secret Rotation_ immediately.

**Secret Rotation:**
1. Revoke the exposed credential immediately (API key, DB password, token).
2. Issue new credentials and update all secrets in GitHub Actions / `.env` files.
3. Rewrite git history to remove the secret: `git filter-repo --path-glob '...'`
   (requires orchestrator approval — this rewrites shared history).
4. Document the incident in a post-mortem bead.

---

## Adding an Exception

### Advisory exception (deny.toml)

Add to the `ignore` list under `[advisories]`:

```toml
[advisories]
ignore = [
  # RUSTSEC-2024-0001: affects only the async variant of foo::bar which we do
  # not use. Fixed in foo 2.1.0 but our dep graph cannot upgrade yet.
  # Tracking: https://github.com/your-org/platform/issues/NNN
  { id = "RUSTSEC-2024-0001", reason = "Not reachable via our call graph (sync path only)" },
]
```

**Required fields:** `id`, `reason`. A tracking issue link is mandatory.

### License exception (deny.toml)

Add to `[[licenses.exceptions]]`:

```toml
[[licenses.exceptions]]
# ring uses ISC + MIT + OpenSSL-like. The OpenSSL-like portion covers only the
# BoringSSL-derived assembly helpers — not our Rust code. Legal approved 2026-01-15.
# Tracking: https://github.com/your-org/platform/issues/NNN
name = "ring"
version = "*"
allow = ["MIT", "ISC", "OpenSSL"]
```

### gitleaks exception (.gitleaks.toml)

Create `.gitleaks.toml` at the workspace root if it does not exist:

```toml
[allowlist]
  description = "False positives — review each entry before adding"

  [[allowlist.commits]]
  # Commit abc123: contains a test fixture RSA key used in unit tests.
  # Not a real credential. See e2e-tests/tests/security_primitives_e2e.rs.
  # Tracking: https://github.com/your-org/platform/issues/NNN
  commit = "abc123def456"
```

Or allowlist by regex if the pattern appears in multiple commits:

```toml
  [[allowlist.regexes]]
  # Example RSA private key header used in test fixtures only. The full key is
  # randomly generated per test run and never persists.
  description = "Test fixture RSA key header"
  regex = "-----BEGIN RSA PRIVATE KEY-----"
  path = "e2e-tests/.*"
```

---

## Running Scans Locally

```bash
# CVE scan
cargo audit

# License + advisory + ban check
cargo deny check

# Secret scan (all committed files, no history)
gitleaks detect --source . --no-git

# Secret scan (git history)
gitleaks detect --source . --log-opts="HEAD"
```

Install tools:

```bash
cargo install cargo-audit --locked
cargo install cargo-deny --locked
# gitleaks: https://github.com/gitleaks/gitleaks#installing
brew install gitleaks  # macOS
```

---

## Weekly Report

The weekly security scan (Monday 03:00 UTC) posts results to the CI dashboard.
If the weekly cargo-deny run finds new advisories that were not present on the
last PR scan, open a tracking issue immediately — do not wait for the next PR.
