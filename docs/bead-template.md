# Bead Description Template

Standard structure for bead descriptions. Ensures any pool agent can execute without guessing.

## Template

```markdown
## What
<Current behavior or problem. What's broken, missing, or suboptimal?>

## Want
<Desired behavior. What should be true when this bead is done?>

## Files
<Files likely involved — paths relative to project root>

## Verify
<Commands to confirm it works. Must use real services — no mocks, no stubs.>

## Skills
<Optional. Hint skills that help: /skill-name>
```

## Requirements by Priority

| Priority | Required Sections |
|----------|-------------------|
| P0-P1    | What + Want + Files + Verify |
| P2       | What + Want + Verify |
| P3-P4    | What only |

## Guidelines

- **What**: Describe the problem, not the solution. Include error messages or symptoms if it's a bug.
- **Want**: Describe the outcome, not the implementation steps. "Users can log in with SSO" not "Add SAML handler to auth middleware."
- **Files**: Best-effort list. Agents will discover more, but this saves initial search time.
- **Verify**: Concrete commands that produce pass/fail results. `cargo test -p foo` or `curl localhost:8080/health`. Not "check that it works."
- **Skills**: Only include if a specific skill meaningfully accelerates the work.

## Example (P1)

```markdown
## What
Mail monitor crashes when agent-mail MCP server restarts. The monitor holds a stale
connection and never reconnects, so the agent stops receiving mail until manually restarted.

## Want
Mail monitor detects connection loss and reconnects automatically within 30 seconds.
No manual intervention needed.

## Files
flywheel_tools/scripts/core/mail-monitor.sh
flywheel_tools/scripts/core/agent-mail-helper.sh

## Verify
1. Kill the MCP mail server: `supervisorctl restart agent-mail`
2. Wait 30s, send a test message: `scripts/agent-mail-helper.sh send GreenRiver "test" "reconnect check"`
3. Confirm delivery: `scripts/agent-mail-helper.sh inbox` (on GreenRiver's session)

## Skills
/bug-hunt
```
