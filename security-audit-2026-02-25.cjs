const fs = require("fs");
const {
  Document, Packer, Paragraph, TextRun, Table, TableRow, TableCell,
  Header, Footer, AlignmentType, HeadingLevel, BorderStyle, WidthType,
  ShadingType, PageNumber, PageBreak, LevelFormat, TabStopType, TabStopPosition,
} = require("docx");

const border = { style: BorderStyle.SINGLE, size: 1, color: "CCCCCC" };
const borders = { top: border, bottom: border, left: border, right: border };
const cellMargins = { top: 60, bottom: 60, left: 100, right: 100 };
const noBorder = { style: BorderStyle.NONE, size: 0 };
const noBorders = { top: noBorder, bottom: noBorder, left: noBorder, right: noBorder };

function headerCell(text, width) {
  return new TableCell({
    borders,
    width: { size: width, type: WidthType.DXA },
    shading: { fill: "1B3A5C", type: ShadingType.CLEAR },
    margins: cellMargins,
    verticalAlign: "center",
    children: [new Paragraph({ children: [new TextRun({ text, bold: true, color: "FFFFFF", font: "Arial", size: 20 })] })],
  });
}

function cell(text, width, opts = {}) {
  const runs = Array.isArray(text) ? text : [new TextRun({ text, font: "Arial", size: 20, ...opts })];
  return new TableCell({
    borders,
    width: { size: width, type: WidthType.DXA },
    shading: opts.fill ? { fill: opts.fill, type: ShadingType.CLEAR } : undefined,
    margins: cellMargins,
    children: [new Paragraph({ children: runs })],
  });
}

function sevCell(severity, width) {
  const colors = {
    CRITICAL: { fill: "C0392B", color: "FFFFFF" },
    HIGH: { fill: "E67E22", color: "FFFFFF" },
    MEDIUM: { fill: "F1C40F", color: "000000" },
    LOW: { fill: "27AE60", color: "FFFFFF" },
    RESOLVED: { fill: "2ECC71", color: "FFFFFF" },
    INFO: { fill: "3498DB", color: "FFFFFF" },
  };
  const c = colors[severity] || colors.INFO;
  return new TableCell({
    borders,
    width: { size: width, type: WidthType.DXA },
    shading: { fill: c.fill, type: ShadingType.CLEAR },
    margins: cellMargins,
    children: [new Paragraph({ alignment: AlignmentType.CENTER, children: [new TextRun({ text: severity, bold: true, color: c.color, font: "Arial", size: 20 })] })],
  });
}

function heading(text, level) {
  return new Paragraph({ heading: level, spacing: { before: 300, after: 150 }, children: [new TextRun({ text, font: "Arial" })] });
}

function para(text, opts = {}) {
  const runs = Array.isArray(text) ? text : [new TextRun({ text, font: "Arial", size: 22, ...opts })];
  return new Paragraph({ spacing: { after: 120 }, children: runs });
}

function boldPara(label, text) {
  return new Paragraph({ spacing: { after: 120 }, children: [
    new TextRun({ text: label, bold: true, font: "Arial", size: 22 }),
    new TextRun({ text, font: "Arial", size: 22 }),
  ]});
}

function bulletItem(text, ref = "bullets") {
  const runs = Array.isArray(text) ? text : [new TextRun({ text, font: "Arial", size: 22 })];
  return new Paragraph({ numbering: { reference: ref, level: 0 }, spacing: { after: 60 }, children: runs });
}

// ───── Document ─────
const doc = new Document({
  styles: {
    default: { document: { run: { font: "Arial", size: 22 } } },
    paragraphStyles: [
      { id: "Heading1", name: "Heading 1", basedOn: "Normal", next: "Normal", quickFormat: true,
        run: { size: 36, bold: true, font: "Arial", color: "1B3A5C" },
        paragraph: { spacing: { before: 360, after: 200 }, outlineLevel: 0 } },
      { id: "Heading2", name: "Heading 2", basedOn: "Normal", next: "Normal", quickFormat: true,
        run: { size: 28, bold: true, font: "Arial", color: "2C5F8A" },
        paragraph: { spacing: { before: 240, after: 150 }, outlineLevel: 1 } },
      { id: "Heading3", name: "Heading 3", basedOn: "Normal", next: "Normal", quickFormat: true,
        run: { size: 24, bold: true, font: "Arial", color: "34495E" },
        paragraph: { spacing: { before: 200, after: 120 }, outlineLevel: 2 } },
    ],
  },
  numbering: {
    config: [
      { reference: "bullets", levels: [{ level: 0, format: LevelFormat.BULLET, text: "\u2022", alignment: AlignmentType.LEFT,
        style: { paragraph: { indent: { left: 720, hanging: 360 } } } }] },
      { reference: "numbers", levels: [{ level: 0, format: LevelFormat.DECIMAL, text: "%1.", alignment: AlignmentType.LEFT,
        style: { paragraph: { indent: { left: 720, hanging: 360 } } } }] },
    ],
  },
  sections: [{
    properties: {
      page: {
        size: { width: 12240, height: 15840 },
        margin: { top: 1440, right: 1200, bottom: 1440, left: 1200 },
      },
    },
    headers: {
      default: new Header({ children: [new Paragraph({
        alignment: AlignmentType.RIGHT,
        children: [new TextRun({ text: "7D-Solutions Platform \u2014 Security Audit", font: "Arial", size: 18, color: "888888", italics: true })],
        border: { bottom: { style: BorderStyle.SINGLE, size: 4, color: "CCCCCC", space: 4 } },
      })] }),
    },
    footers: {
      default: new Footer({ children: [new Paragraph({
        alignment: AlignmentType.CENTER,
        children: [
          new TextRun({ text: "Confidential \u2014 Page ", font: "Arial", size: 18, color: "888888" }),
          new TextRun({ children: [PageNumber.CURRENT], font: "Arial", size: 18, color: "888888" }),
        ],
      })] }),
    },
    children: [
      // ══════ TITLE PAGE ══════
      new Paragraph({ spacing: { before: 2400 } }),
      new Paragraph({ alignment: AlignmentType.CENTER, spacing: { after: 200 }, children: [
        new TextRun({ text: "SECURITY AUDIT REPORT", font: "Arial", size: 52, bold: true, color: "1B3A5C" }),
      ] }),
      new Paragraph({ alignment: AlignmentType.CENTER, spacing: { after: 100 }, children: [
        new TextRun({ text: "7D-Solutions Platform", font: "Arial", size: 36, color: "2C5F8A" }),
      ] }),
      new Paragraph({ alignment: AlignmentType.CENTER, spacing: { after: 600 },
        border: { bottom: { style: BorderStyle.SINGLE, size: 6, color: "1B3A5C", space: 8 } },
        children: [new TextRun({ text: "February 25, 2026", font: "Arial", size: 26, color: "555555" })] }),
      new Paragraph({ spacing: { before: 400 } }),
      new Table({
        width: { size: 5000, type: WidthType.DXA },
        columnWidths: [2000, 3000],
        rows: [
          new TableRow({ children: [
            new TableCell({ borders: noBorders, width: { size: 2000, type: WidthType.DXA }, children: [new Paragraph({ children: [new TextRun({ text: "Classification:", bold: true, font: "Arial", size: 22, color: "333333" })] })] }),
            new TableCell({ borders: noBorders, width: { size: 3000, type: WidthType.DXA }, children: [new Paragraph({ children: [new TextRun({ text: "CONFIDENTIAL", font: "Arial", size: 22, color: "C0392B", bold: true })] })] }),
          ] }),
          new TableRow({ children: [
            new TableCell({ borders: noBorders, width: { size: 2000, type: WidthType.DXA }, children: [new Paragraph({ children: [new TextRun({ text: "Scope:", bold: true, font: "Arial", size: 22, color: "333333" })] })] }),
            new TableCell({ borders: noBorders, width: { size: 3000, type: WidthType.DXA }, children: [new Paragraph({ children: [new TextRun({ text: "Full codebase audit", font: "Arial", size: 22 })] })] }),
          ] }),
          new TableRow({ children: [
            new TableCell({ borders: noBorders, width: { size: 2000, type: WidthType.DXA }, children: [new Paragraph({ children: [new TextRun({ text: "Platform:", bold: true, font: "Arial", size: 22, color: "333333" })] })] }),
            new TableCell({ borders: noBorders, width: { size: 3000, type: WidthType.DXA }, children: [new Paragraph({ children: [new TextRun({ text: "Rust (Axum) + Next.js", font: "Arial", size: 22 })] })] }),
          ] }),
          new TableRow({ children: [
            new TableCell({ borders: noBorders, width: { size: 2000, type: WidthType.DXA }, children: [new Paragraph({ children: [new TextRun({ text: "Modules:", bold: true, font: "Arial", size: 22, color: "333333" })] })] }),
            new TableCell({ borders: noBorders, width: { size: 3000, type: WidthType.DXA }, children: [new Paragraph({ children: [new TextRun({ text: "19 service modules + 9 platform crates", font: "Arial", size: 22 })] })] }),
          ] }),
        ],
      }),

      new Paragraph({ children: [new PageBreak()] }),

      // ══════ EXECUTIVE SUMMARY ══════
      heading("1. Executive Summary", HeadingLevel.HEADING_1),
      para("This report presents the findings of a comprehensive security audit of the 7D-Solutions Platform conducted on February 25, 2026. The audit covers authentication, authorization, secrets management, input validation, API exposure, dependency security, and CORS configuration across all 19 service modules and 9 platform crates."),
      para("The platform demonstrates strong security foundations: Argon2id password hashing with configurable parameters and CPU throttling, RS256 JWT with zero-downtime key rotation, PII redaction utilities, per-email rate limiting with account lockout, DB-backed session seat limits, and constant-time webhook signature verification."),
      para([
        new TextRun({ text: "Since the previous audit (Feb 24, 2026), three critical findings have been resolved: ", font: "Arial", size: 22 }),
        new TextRun({ text: "AR/TTP/party modules now have JWT verification middleware, ", font: "Arial", size: 22, italics: true }),
        new TextRun({ text: "and the legacy no-op AuthzLayer has been removed from affected modules. SQL injection in projections admin has been mitigated with allowlist validation.", font: "Arial", size: 22 }),
      ]),
      para([
        new TextRun({ text: "Remaining concerns center on: ", font: "Arial", size: 22 }),
        new TextRun({ text: "CORS wildcard origins across all 19 modules, hardcoded tenant identifiers in the AR module, and global IP-based rate limiting being disabled.", font: "Arial", size: 22, bold: true }),
      ]),

      // ══════ SUMMARY TABLE ══════
      heading("2. Findings Summary", HeadingLevel.HEADING_1),
      new Table({
        width: { size: 9840, type: WidthType.DXA },
        columnWidths: [600, 1200, 5040, 3000],
        rows: [
          new TableRow({ children: [headerCell("#", 600), headerCell("Severity", 1200), headerCell("Finding", 5040), headerCell("Status", 3000)] }),
          new TableRow({ children: [cell("C1", 600), sevCell("RESOLVED", 1200), cell("AR/TTP/party missing JWT middleware", 5040), cell("Fixed 2026-02-24", 3000, { fill: "E8F5E9" })] }),
          new TableRow({ children: [cell("C2", 600), sevCell("RESOLVED", 1200), cell("Legacy AuthzLayer is a no-op stub", 5040), cell("Removed 2026-02-24", 3000, { fill: "E8F5E9" })] }),
          new TableRow({ children: [cell("H1", 600), sevCell("HIGH", 1200), cell("CORS wildcard origins on all 19 modules", 5040), cell("Open", 3000, { fill: "FFF3E0" })] }),
          new TableRow({ children: [cell("H2", 600), sevCell("HIGH", 1200), cell("AR module: 41 hardcoded tenant IDs", 5040), cell("Open", 3000, { fill: "FFF3E0" })] }),
          new TableRow({ children: [cell("H3", 600), sevCell("HIGH", 1200), cell("Global IP rate limiter disabled", 5040), cell("Open", 3000, { fill: "FFF3E0" })] }),
          new TableRow({ children: [cell("M1", 600), sevCell("MEDIUM", 1200), cell("Service-to-service auth uses shared symmetric HMAC", 5040), cell("Open", 3000)] }),
          new TableRow({ children: [cell("M2", 600), sevCell("MEDIUM", 1200), cell("In-memory rate limiter state not shared across replicas", 5040), cell("Open", 3000)] }),
          new TableRow({ children: [cell("M3", 600), sevCell("MEDIUM", 1200), cell("Nginx proxy listens on HTTP only (no TLS config)", 5040), cell("Open", 3000)] }),
          new TableRow({ children: [cell("M4", 600), sevCell("MEDIUM", 1200), cell("Docker-compose uses default/weak DB passwords", 5040), cell("Open", 3000)] }),
          new TableRow({ children: [cell("M5", 600), sevCell("RESOLVED", 1200), cell("SQL injection via dynamic table names in projections", 5040), cell("Allowlist added", 3000, { fill: "E8F5E9" })] }),
          new TableRow({ children: [cell("L1", 600), sevCell("LOW", 1200), cell(".env contains development RSA private key", 5040), cell("Gitignored", 3000)] }),
          new TableRow({ children: [cell("L2", 600), sevCell("LOW", 1200), cell("Docker data services expose ports to host", 5040), cell("Dev-only", 3000)] }),
          new TableRow({ children: [cell("L3", 600), sevCell("LOW", 1200), cell("Password denylist is minimal (5 entries)", 5040), cell("Open", 3000)] }),
          new TableRow({ children: [cell("L4", 600), sevCell("INFO", 1200), cell("No cargo audit / npm audit in CI pipeline", 5040), cell("Open", 3000)] }),
        ],
      }),

      new Paragraph({ children: [new PageBreak()] }),

      // ══════ DETAILED FINDINGS ══════
      heading("3. Detailed Findings", HeadingLevel.HEADING_1),

      // H1
      heading("H1. CORS Wildcard Origins on All 19 Modules", HeadingLevel.HEADING_2),
      boldPara("Severity: ", "HIGH"),
      para("Every module in the platform defaults to allow_origin(Any) when the CORS_ORIGINS environment variable is set to \"*\" (or unset, as the default fallback is wildcard). While origins are configurable via CORS_ORIGINS, the default is insecure. Additionally, all modules use allow_methods(Any) and allow_headers(Any) unconditionally, regardless of the origin setting."),
      boldPara("Impact: ", "Any website can make authenticated cross-origin requests to the platform APIs if credentials are included. Combined with JWT Bearer auth (which is not cookie-based), the practical risk is somewhat mitigated, but this still violates defense-in-depth."),
      boldPara("Affected modules: ", "AR, AP, GL, inventory, payments, subscriptions, notifications, consolidation, timekeeping, treasury, fixed-assets, maintenance, integrations, party, ttp, shipping-receiving, pdf-editor, reporting."),
      boldPara("Recommendation: ", "Set CORS_ORIGINS to specific allowed origins in all environments. Change the default fallback from \"*\" to an empty list (deny all cross-origin) when the env var is not set. Restrict allow_methods and allow_headers to only what the API actually uses."),

      // H2
      heading("H2. AR Module: 41 Hardcoded Tenant Identifiers", HeadingLevel.HEADING_2),
      boldPara("Severity: ", "HIGH"),
      para("The AR module uses hardcoded \"test-app\" and \"default-tenant\" strings as app_id / tenant scoping in route handlers. Even though JWT middleware is now correctly wired (fixed in C1), route handlers extract tenant_id from request bodies or use hardcoded defaults rather than from the verified JWT claims."),
      boldPara("Impact: ", "Tenant data isolation depends on callers providing the correct tenant_id. A malicious client with a valid JWT could query another tenant's data by submitting a different tenant_id in the request body."),
      boldPara("Remaining occurrences: ", "41 across 3 files in modules/ar/src/."),
      boldPara("Recommendation: ", "Extract tenant_id exclusively from VerifiedClaims (set by JWT middleware). Reject requests where the body tenant_id differs from the token's tenant_id."),

      // H3
      heading("H3. Global IP Rate Limiter Disabled", HeadingLevel.HEADING_2),
      boldPara("Severity: ", "HIGH"),
      para("The per-IP governor rate limiter in identity-auth is commented out with \"TODO: Re-enable when tower_governor works with axum 0.7\". The code references tower_governor 0.8 in Cargo.toml but the actual limiter layer is never applied to the router."),
      boldPara("Impact: ", "While per-email rate limiting still works for login/register/forgot-password flows, there is no global IP-based rate limiting. An attacker could enumerate valid emails or probe endpoints at high volume from a single IP without being throttled."),
      boldPara("Recommendation: ", "Re-enable the governor layer or implement an equivalent using the existing governor crate (which is already a dependency). The keyed rate limiters cover auth endpoints, but health, metrics, JWKS, and any future endpoints have zero rate limiting."),

      // M1
      heading("M1. Symmetric Service-to-Service Auth", HeadingLevel.HEADING_2),
      boldPara("Severity: ", "MEDIUM"),
      para("Service-to-service authentication (platform/security/src/service_auth.rs) uses HMAC-SHA256 with a shared SERVICE_AUTH_SECRET environment variable. Any service with this secret can impersonate any other service. The service_name claim in the token is self-asserted and not independently verified."),
      boldPara("Recommendation: ", "Consider per-service keys or asymmetric signing for better isolation. At minimum, implement a service name allowlist for each consumer endpoint."),

      // M2
      heading("M2. In-Memory Rate Limiter Not Shared Across Replicas", HeadingLevel.HEADING_2),
      boldPara("Severity: ", "MEDIUM"),
      para("The KeyedLimiters in identity-auth uses DashMap (in-memory) for rate limit state. Docker-compose runs two auth replicas behind nginx round-robin. An attacker can effectively double their allowed rate by alternating between replicas."),
      boldPara("Recommendation: ", "Use Redis-backed rate limiting or a sticky-session strategy to ensure rate limit state is consistent."),

      // M3
      heading("M3. Nginx Proxy Listens HTTP Only", HeadingLevel.HEADING_2),
      boldPara("Severity: ", "MEDIUM"),
      para("The nginx configuration (nginx/auth-nginx.conf) listens on port 80 with no TLS configuration. While TLS may be terminated upstream, the config as written allows unencrypted traffic to the auth service including JWT tokens and passwords."),
      boldPara("Recommendation: ", "Add TLS termination to nginx or document that TLS is handled by an external load balancer. Add HSTS headers."),

      // M4
      heading("M4. Docker-Compose Uses Default/Weak DB Passwords", HeadingLevel.HEADING_2),
      boldPara("Severity: ", "MEDIUM"),
      para("All 18 PostgreSQL instances in docker-compose.data.yml use weak default passwords like \"auth_pass\", \"ar_pass\", etc. via shell variable defaults (${AUTH_POSTGRES_PASSWORD:-auth_pass}). These are exposed on mapped host ports (5433-5460+). While intended for development, these defaults could persist to staging if .env files are not properly configured."),
      boldPara("Recommendation: ", "Remove default password fallbacks in docker-compose files. Require explicit env vars for all database passwords. Use Docker secrets or Vault in staging/production."),

      new Paragraph({ children: [new PageBreak()] }),

      // ══════ POSITIVE FINDINGS ══════
      heading("4. Positive Security Findings", HeadingLevel.HEADING_1),
      para("The platform has many well-implemented security controls:"),

      new Table({
        width: { size: 9840, type: WidthType.DXA },
        columnWidths: [3000, 6840],
        rows: [
          new TableRow({ children: [headerCell("Area", 3000), headerCell("Assessment", 6840)] }),
          new TableRow({ children: [cell("Password Hashing", 3000, { bold: true }), cell("Argon2id with configurable memory/iterations/parallelism. CPU-bound hash operations throttled via semaphore (max_concurrent_hashes). OsRng for salt generation.", 6840)] }),
          new TableRow({ children: [cell("JWT Architecture", 3000, { bold: true }), cell("RS256 with RSA-2048 keys. Validates exp, iss (auth-rs), and aud (7d-platform). Supports zero-downtime key rotation with prev_key overlap. JWKS endpoint for key discovery.", 6840)] }),
          new TableRow({ children: [cell("PII Redaction", 3000, { bold: true }), cell("Redacted<T> wrapper prevents Display/Debug leakage. Dedicated redact_email(), redact_partial(), redact_name() helpers. Comprehensive PII field inventory documented.", 6840)] }),
          new TableRow({ children: [cell("Auth Rate Limiting", 3000, { bold: true }), cell("Per-email login/register limits, per-token refresh limits, per-IP forgot/reset limits. Configurable via env vars. Retry-After headers on 429.", 6840)] }),
          new TableRow({ children: [cell("Account Lockout", 3000, { bold: true }), cell("Progressive failed login counting. Temporary lock_until with configurable threshold (default: 10 attempts) and duration (default: 15 min).", 6840)] }),
          new TableRow({ children: [cell("Webhook Verification", 3000, { bold: true }), cell("HMAC-SHA256 with constant-time comparison. Stripe-compatible timestamp replay protection (300s tolerance). Generic adapter for custom headers.", 6840)] }),
          new TableRow({ children: [cell("Session Management", 3000, { bold: true }), cell("DB-backed seat leases with advisory locks for atomic enforcement. Tenant entitlement-based concurrent user limits. Refresh token rotation with replay detection.", 6840)] }),
          new TableRow({ children: [cell("Tenant Lifecycle", 3000, { bold: true }), cell("Suspended/canceled tenants denied login. Past-due tenants denied new login but can refresh. Fail-closed on registry unavailability.", 6840)] }),
          new TableRow({ children: [cell("unsafe_code Lint", 3000, { bold: true }), cell("Workspace-level deny(unsafe_code) in Cargo.toml. Unsafe only exists in test code for env var mutation (Rust 1.83+ requirement).", 6840)] }),
          new TableRow({ children: [cell("Parameterized SQL", 3000, { bold: true }), cell("All user-facing queries use sqlx bind parameters ($1, $2). Dynamic table names in projections now validated against allowlist.", 6840)] }),
          new TableRow({ children: [cell("Authz Middleware", 3000, { bold: true }), cell("ClaimsLayer + RequirePermissionsLayer pattern: extract JWT claims then enforce per-route permission strings. 401 for missing token, 403 for insufficient perms.", 6840)] }),
          new TableRow({ children: [cell("Password Policy", 3000, { bold: true }), cell("Min 12 chars, require upper + lower + digit. Denylist of common passwords. Validation before rate limit check or hashing.", 6840)] }),
        ],
      }),

      new Paragraph({ children: [new PageBreak()] }),

      // ══════ REMEDIATION PRIORITIES ══════
      heading("5. Remediation Priority Matrix", HeadingLevel.HEADING_1),
      para("Recommended order of remediation based on risk and effort:"),

      new Table({
        width: { size: 9840, type: WidthType.DXA },
        columnWidths: [600, 1000, 4240, 2000, 2000],
        rows: [
          new TableRow({ children: [headerCell("#", 600), headerCell("Priority", 1000), headerCell("Action", 4240), headerCell("Effort", 2000), headerCell("Risk Reduction", 2000)] }),
          new TableRow({ children: [cell("1", 600), sevCell("HIGH", 1000), cell("Set CORS_ORIGINS to specific origins in all environments", 4240), cell("Low (env config)", 2000), cell("High", 2000)] }),
          new TableRow({ children: [cell("2", 600), sevCell("HIGH", 1000), cell("Extract tenant_id from JWT claims in AR route handlers", 4240), cell("Medium (41 sites)", 2000), cell("High", 2000)] }),
          new TableRow({ children: [cell("3", 600), sevCell("HIGH", 1000), cell("Re-enable global IP rate limiting in identity-auth", 4240), cell("Low (uncomment)", 2000), cell("Medium", 2000)] }),
          new TableRow({ children: [cell("4", 600), sevCell("MEDIUM", 1000), cell("Add TLS to nginx or document upstream termination", 4240), cell("Low", 2000), cell("Medium", 2000)] }),
          new TableRow({ children: [cell("5", 600), sevCell("MEDIUM", 1000), cell("Remove default DB password fallbacks in docker-compose", 4240), cell("Low", 2000), cell("Medium", 2000)] }),
          new TableRow({ children: [cell("6", 600), sevCell("MEDIUM", 1000), cell("Move to per-service auth keys or asymmetric signing", 4240), cell("Medium", 2000), cell("Medium", 2000)] }),
          new TableRow({ children: [cell("7", 600), sevCell("MEDIUM", 1000), cell("Implement shared rate limit state (Redis)", 4240), cell("Medium", 2000), cell("Low-Medium", 2000)] }),
          new TableRow({ children: [cell("8", 600), sevCell("LOW", 1000), cell("Add cargo audit and npm audit to CI", 4240), cell("Low", 2000), cell("Low", 2000)] }),
          new TableRow({ children: [cell("9", 600), sevCell("LOW", 1000), cell("Expand password denylist (load from file or HaveIBeenPwned)", 4240), cell("Low", 2000), cell("Low", 2000)] }),
        ],
      }),

      new Paragraph({ spacing: { before: 400 } }),

      // ══════ RESOLVED SINCE LAST AUDIT ══════
      heading("6. Resolved Since Previous Audit (Feb 24)", HeadingLevel.HEADING_1),
      para("The following critical and high-severity findings from the February 24, 2026 audit have been addressed:"),

      bulletItem([
        new TextRun({ text: "C1 (CRITICAL): ", bold: true, font: "Arial", size: 22 }),
        new TextRun({ text: "AR, TTP, and party modules now wire optional_claims_mw for JWT verification, matching the GL/inventory pattern. Confirmed in main.rs for all three modules.", font: "Arial", size: 22 }),
      ]),
      bulletItem([
        new TextRun({ text: "C2 (CRITICAL): ", bold: true, font: "Arial", size: 22 }),
        new TextRun({ text: "Legacy AuthzLayer::from_env() has been removed from AR, TTP, and payments modules. The no-op stub no longer creates a false sense of security.", font: "Arial", size: 22 }),
      ]),
      bulletItem([
        new TextRun({ text: "H2/M5 (MEDIUM-HIGH): ", bold: true, font: "Arial", size: 22 }),
        new TextRun({ text: "SQL injection risk in projections admin endpoint mitigated by validate_projection_name() allowlist check. Test confirms injection attempts are rejected.", font: "Arial", size: 22 }),
      ]),
      bulletItem([
        new TextRun({ text: "M3 (MEDIUM): ", bold: true, font: "Arial", size: 22 }),
        new TextRun({ text: "deny(unsafe_code) is now set at the workspace level in Cargo.toml. Only test code uses unsafe (for env var mutation per Rust 1.83+ safety requirements).", font: "Arial", size: 22 }),
      ]),

      new Paragraph({ spacing: { before: 400 } }),
      heading("7. Methodology", HeadingLevel.HEADING_1),
      para("This audit was performed through static analysis of the full source code repository. The following areas were examined:"),
      bulletItem("Authentication flows: registration, login, token refresh, logout, password reset"),
      bulletItem("Authorization middleware: JWT verification, RBAC permission enforcement"),
      bulletItem("Secrets and configuration: .env files, docker-compose environment variables, key management"),
      bulletItem("Input validation: SQL query construction, email validation, password policy"),
      bulletItem("API exposure: CORS configuration, rate limiting, body size limits"),
      bulletItem("Dependency security: Cargo.toml dependency versions, workspace lints"),
      bulletItem("Network architecture: nginx proxy config, service-to-service auth, port exposure"),
      bulletItem("Comparison with previous audit (Feb 24) to verify remediation of critical findings"),

      new Paragraph({ spacing: { before: 200 } }),
      new Paragraph({ border: { top: { style: BorderStyle.SINGLE, size: 4, color: "1B3A5C", space: 8 } }, spacing: { before: 400 }, children: [
        new TextRun({ text: "End of Report", italics: true, font: "Arial", size: 20, color: "888888" }),
      ] }),
    ],
  }],
});

Packer.toBuffer(doc).then(buf => {
  fs.writeFileSync("/sessions/gallant-brave-wozniak/mnt/7D-Solutions Platform/security-audit-2026-02-25.docx", buf);
  console.log("Report written successfully");
});
