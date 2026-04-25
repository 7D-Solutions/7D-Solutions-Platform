# pdfium CVE / Patch SLA

**Bead:** bd-53hfo  
**Audience:** Security auditors, Phase 0A IvoryHollow review  
**Owner:** Platform Engineering  
**Last reviewed:** 2026-04-25

---

## 1. What is pdfium?

`pdfium` is Google's PDF rendering library (extracted from Chromium). The pdf-editor service bundles a pre-compiled native binary (`libpdfium.so` in production, `libpdfium.dylib` on macOS dev hosts). Because it is a vendored binary — not a crate on crates.io — it is outside the scope of `cargo-audit` and `cargo-deny`. It requires its own monitoring and patch process.

---

## 2. Vendor Monitoring

| Source | What to watch | Cadence |
|--------|--------------|---------|
| [NVD search — pdfium](https://nvd.nist.gov/vuln/search/results?query=pdfium) | New CVEs tagged with `pdfium` | Weekly (Monday) |
| [Chromium security advisories](https://chromereleases.googleblog.com/) | Stable channel release notes — look for "pdfium" in the list of fixes | Weekly (Monday) |
| [pdfium-binaries releases](https://github.com/bblanchon/pdfium-binaries/releases) | Pre-built binaries tracking upstream Chromium; release notes include upstream CVE fixes | Weekly (Monday) |
| GitHub Dependabot / OSV | If OSV adds a pdfium advisory it will surface here | Automatic |

The weekly review is owned by the engineer on rotation. Record findings in a bead titled `[pdfium] CVE-XXXX-XXXXX review` even if the finding is "no action required."

---

## 3. Severity SLA

Follows the platform-wide patching cadence defined in `docs/operations/patching-cadence.md`, with one pdfium-specific tightening at Critical:

| Severity | CVSS Range | Patch Applied Within |
|----------|-----------|---------------------|
| Critical | 9.0–10.0 | **14 calendar days** of vendor release |
| High | 7.0–8.9 | 30 calendar days |
| Medium | 4.0–6.9 | 60 calendar days |
| Low | 0.1–3.9 | 90 calendar days |

"Applied within N days" = patched image deployed to production within N days of NVD publish date or pdfium-binaries release, whichever is earlier.

---

## 4. Patch Rollout Procedure

### Step 1 — Identify the patched binary

1. Find the pdfium-binaries release that includes the fix:
   ```
   https://github.com/bblanchon/pdfium-binaries/releases
   ```
2. Note the release tag (e.g., `chromium/6XXX`) and the associated Chromium version.
3. Download `pdfium-linux-x64.tgz` (and `pdfium-mac-x64.tgz` for dev hosts).

### Step 2 — Update the vendored binary

```bash
# Download and replace the production binary
curl -L https://github.com/bblanchon/pdfium-binaries/releases/download/<TAG>/pdfium-linux-x64.tgz \
  | tar -xz --strip-components=1 -C modules/pdf-editor/ lib/libpdfium.so

# Update the macOS dev binary
curl -L https://github.com/bblanchon/pdfium-binaries/releases/download/<TAG>/pdfium-mac-x64.tgz \
  | tar -xz --strip-components=1 -C modules/pdf-editor/ lib/libpdfium.dylib
```

### Step 3 — Record the version

Update `modules/pdf-editor/REVISIONS.md` with:
- Chromium/pdfium-binaries release tag
- CVE(s) addressed
- Date

### Step 4 — Run integration tests

```bash
./scripts/cargo-slot.sh test -p pdf-editor
```

All tests must pass before committing.

### Step 5 — Commit and deploy

Commit with bead prefix and push. The cross-watcher picks up the new binary on the next build cycle (~5 minutes). Verify the running container serves PDFs correctly with a smoke test before declaring the patch complete.

---

## 5. Rollback Path

If the patched binary causes a regression:

1. Identify the previous binary commit hash:
   ```bash
   git log --oneline modules/pdf-editor/libpdfium.so
   ```
2. Restore the previous binary:
   ```bash
   git checkout <previous-commit> -- modules/pdf-editor/libpdfium.so modules/pdf-editor/libpdfium.dylib
   ```
3. Create a bead documenting the regression and the deferred patch per the exception process in `patching-cadence.md` §5.
4. The rollback commit must be pushed and confirmed deployed before the SLA clock is considered paused.

---

## 6. Container Image Scanning

A dedicated CI job (`pdf-editor-image-scan.yml`) builds the pdf-editor container image and runs **Trivy** against it on every push and pull request. The job fails if any **CRITICAL** CVE is found. This catches OS-layer and glibc CVEs that `cargo-audit` does not cover, including any CVEs associated with the bundled `libpdfium.so`.

See `.github/workflows/pdf-editor-image-scan.yml` for the full workflow.

---

## 7. Exception Process

A patch may be deferred beyond its SLA window only when applying it would cause demonstrable service disruption and no mitigating control is available. Follow the exception process defined in `docs/operations/patching-cadence.md` §5.

---

## 8. Evidence for Auditors

| Artifact | Location |
|----------|---------|
| Weekly monitoring records | Bead history, labeled `[pdfium]` |
| Binary version in use | `modules/pdf-editor/REVISIONS.md` |
| Trivy scan results | GitHub Actions — `pdf-editor-image-scan` job per PR/push |
| Active deferrals | `docs/operations/patch-deferrals.md` |

---

*This document is reviewed monthly and updated whenever the pdfium binary is replaced or SLAs change. For questions, contact the Platform Engineering team.*
