# Change Control

**Version:** 1.0  
**Status:** Active  
**Last Updated:** 2026-02-11

## Overview

This document defines the process for proposing, reviewing, and approving changes to the 7D Solutions Platform.

## Change Categories

### Category 1: Low Risk
**Examples:**
- Bug fixes
- Documentation updates
- Test improvements
- Internal refactoring (no API changes)
- PATCH version releases

**Process:**
- Standard PR review
- 1 approval required
- Merge after CI passes

### Category 2: Medium Risk
**Examples:**
- New features (backward compatible)
- New optional API endpoints
- MINOR version releases
- Configuration changes

**Process:**
- PR with detailed description
- 1-2 approvals required
- CI + contract tests must pass
- Notify affected teams

### Category 3: High Risk
**Examples:**
- Breaking API changes (MAJOR version)
- Database schema changes
- Platform-wide changes
- Security changes

**Process:**
- RFC document required
- Architecture review
- Multiple approvals (2+ owners)
- Minimum notice period (2-4 weeks)
- Migration plan required

## Pull Request Process

### 1. Create Branch

```bash
# Feature branch
git checkout -b feature/billing-recurring-invoices

# Bug fix branch
git checkout -b fix/billing-tax-calculation

# Breaking change branch
git checkout -b breaking/billing-v3-api
```

### 2. Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

**Types:**
- `feat:` - New feature
- `fix:` - Bug fix
- `docs:` - Documentation
- `refactor:` - Code refactoring
- `test:` - Test changes
- `chore:` - Build/tooling changes

**Breaking changes:**
```
feat(billing)!: remove deprecated invoice API

BREAKING CHANGE: Removed /api/v1/invoices endpoint.
Use /api/v2/invoices instead.

Refs: #123
```

### 3. PR Template

```markdown
## Description
[Clear description of what changed and why]

## Type of Change
- [ ] Bug fix (non-breaking)
- [ ] New feature (non-breaking)
- [ ] Breaking change
- [ ] Documentation update

## Testing
- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] Contract tests pass
- [ ] Manual testing completed

## Checklist
- [ ] Code follows style guidelines
- [ ] Self-review completed
- [ ] Documentation updated
- [ ] No new warnings
- [ ] Tests added/updated
- [ ] Changelog updated (if applicable)

## Related Issues
Closes #123
Refs #456

## Screenshots (if applicable)
[Add screenshots for UI changes]

## Migration Notes (if breaking change)
[How to migrate from old to new version]
```

### 4. Review Process

**Reviewer responsibilities:**

**Code Quality:**
- [ ] Logic is correct
- [ ] Code is readable
- [ ] No obvious bugs
- [ ] Error handling is appropriate
- [ ] Tests are sufficient

**Architecture:**
- [ ] Follows layering rules
- [ ] No cross-module imports
- [ ] Contracts are honored
- [ ] No circular dependencies

**Security:**
- [ ] No SQL injection
- [ ] No XSS vulnerabilities
- [ ] Secrets not hardcoded
- [ ] Input validation present

**Performance:**
- [ ] No N+1 queries
- [ ] Appropriate caching
- [ ] No unnecessary loops
- [ ] Database indexes considered

### 5. Approval Requirements

**Standard changes:**
- 1 approval from module owner OR
- 2 approvals from team members

**Breaking changes:**
- 2+ approvals from module owners
- 1 approval from tech lead
- All contract tests passing

**Platform changes:**
- 2 approvals from platform team
- Approval from affected module owners (if breaking)

### 6. Merge Requirements

Before merging:
- [ ] All CI checks pass
- [ ] Required approvals obtained
- [ ] No merge conflicts
- [ ] Branch is up-to-date with main
- [ ] Documentation updated

## Request for Comments (RFC)

For significant changes, create an RFC before implementation.

### When to Write RFC

**Required for:**
- Breaking changes (MAJOR version)
- New modules
- Platform architecture changes
- Cross-cutting concerns

**Optional for:**
- Large features (> 1 week effort)
- Design trade-offs need discussion

### RFC Template

```markdown
# RFC-{NUMBER}: {TITLE}

**Author:** [Your name]
**Status:** [Draft | Review | Accepted | Rejected]
**Created:** YYYY-MM-DD

## Summary
[1-2 paragraph overview]

## Motivation
[Why is this change needed?]

## Detailed Design
[Technical details, architecture diagrams]

## Alternatives Considered
[What other approaches were considered?]

## Migration Strategy
[How will existing code/users migrate?]

## Unresolved Questions
[What needs to be figured out?]

## Timeline
[Estimated implementation timeline]
```

### RFC Process

1. **Draft** - Author writes RFC
2. **Review** - Post to #architecture channel
3. **Discussion** - 1-2 week comment period
4. **Decision** - Accept, reject, or request changes
5. **Implementation** - If accepted, create issues

## Emergency Changes

For production incidents:

**Process:**
1. Create hotfix branch
2. Implement minimal fix
3. Create PR (marked URGENT)
4. 1 approval required (can be post-merge)
5. Deploy to production
6. Follow-up with proper fix

**Example:**
```bash
git checkout -b hotfix/billing-critical-bug
# Fix bug
git commit -m "fix(billing)!: critical tax calculation bug"
git push
# Create PR, deploy immediately
# Get approval post-merge
```

## Rollback Process

If a change causes issues:

1. **Immediate rollback**
   ```bash
   git revert <commit-sha>
   git push
   # Deploy revert
   ```

2. **Investigate root cause**
3. **Create proper fix**
4. **Re-deploy with fix**

## Deprecation Process

When deprecating features:

1. **Announce** (Version N)
   - Add deprecation notice to code
   - Update documentation
   - Notify affected teams

2. **Grace period** (Versions N+1, N+2)
   - Feature still works
   - Warnings in logs
   - Migration guide available

3. **Remove** (Version N+3 - MAJOR bump)
   - Feature removed
   - BREAKING CHANGE in changelog

**Minimum grace period:**
- Public APIs: 6 months (2 MINOR versions)
- Internal APIs: 3 months (1 MINOR version)

## Change Communication

### Slack Notifications

**#engineering:**
- All PRs merged to main
- Breaking changes
- Emergency fixes

**#architecture:**
- RFCs posted
- Breaking changes
- Platform changes

**Module channels:**
- Changes to specific modules
- Release announcements

### Email Notifications

**Weekly digest:**
- Summary of merged changes
- Upcoming breaking changes
- Deprecation notices

## Metrics

Track these metrics:

- Average PR size (lines changed)
- Time to first review
- Time to merge
- Rework rate (PRs requiring changes)
- Rollback rate

**Targets:**
- Time to first review: < 4 hours
- Time to merge: < 24 hours
- Rollback rate: < 2%

## See Also

- [Code Ownership](CODE-OWNERSHIP.md) - Who approves what
- [Release Policy](RELEASE-POLICY.md) - Release process
- [CI Guardrails](../architecture/CI-GUARDRAILS.md) - Automated checks
