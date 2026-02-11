# Release Policy

**Version:** 1.0  
**Status:** Active  
**Last Updated:** 2026-02-11

## Overview

This document defines the release process, cadence, and policies for modules, platform components, and products in the 7D Solutions Platform.

## Release Types

### Scheduled Releases

**Module releases:**
- Cadence: Every 2 weeks (biweekly)
- Day: Tuesday, 10 AM PT
- Contains: Accumulated features and fixes

**Platform releases:**
- Cadence: Monthly
- Day: First Tuesday of month
- Contains: Platform improvements, security updates

**Product releases:**
- Cadence: Monthly
- Day: Second Tuesday of month
- Contains: New features, module updates

### Hotfix Releases

**When:**
- Critical bugs
- Security vulnerabilities
- Data corruption issues

**Process:**
- On-demand, immediate
- Skip normal release cycle
- PATCH version bump

### Emergency Releases

**When:**
- Production down
- Security breach
- Data loss risk

**Process:**
- Bypass normal approvals
- Deploy immediately
- Post-mortem required

## Release Process

### 1. Pre-Release

#### Code Freeze
**Timeline:** 2 days before release

- No new features merged
- Only bug fixes allowed
- Final testing begins

#### Checklist
- [ ] All tests passing
- [ ] Contract tests passing
- [ ] Performance tests passing
- [ ] Security scan passing
- [ ] Documentation updated
- [ ] CHANGELOG.md updated
- [ ] Migration guide written (if breaking)

### 2. Version Bump

```bash
# For module
cd modules/billing
npm version minor -m "Release v%s"
# or: npm version major / patch

# For platform component
cd platform/identity
# Update Cargo.toml version
git commit -m "Release v2.1.0"
git tag platform-identity-v2.1.0
```

### 3. Build Artifacts

```bash
# Build Docker image
docker build -t ghcr.io/7d-solutions/billing:2.1.0 .
docker tag ghcr.io/7d-solutions/billing:2.1.0 ghcr.io/7d-solutions/billing:2
docker tag ghcr.io/7d-solutions/billing:2.1.0 ghcr.io/7d-solutions/billing:latest

# Push to registry
docker push ghcr.io/7d-solutions/billing:2.1.0
docker push ghcr.io/7d-solutions/billing:2
docker push ghcr.io/7d-solutions/billing:latest
```

### 4. Deploy to Staging

```bash
# Update staging environment
kubectl set image deployment/billing billing=ghcr.io/7d-solutions/billing:2.1.0 -n staging

# Run smoke tests
tools/ci/smoke-tests.sh staging
```

### 5. Deploy to Production

**Timeline:** 24 hours after staging deployment

```bash
# Deploy with rolling update
kubectl set image deployment/billing billing=ghcr.io/7d-solutions/billing:2.1.0 -n production

# Monitor
kubectl rollout status deployment/billing -n production

# Verify
tools/ci/smoke-tests.sh production
```

### 6. Post-Release

- [ ] Verify deployment health
- [ ] Monitor error rates
- [ ] Check performance metrics
- [ ] Update release notes
- [ ] Announce in #engineering
- [ ] Update customer documentation

## Versioning

Follow [Semantic Versioning 2.0.0](https://semver.org/):

### MAJOR (X.0.0)
- Breaking API changes
- Incompatible changes

**Requires:**
- RFC document
- Migration guide
- 30-day notice
- Approval from tech lead

### MINOR (x.Y.0)
- New backward-compatible features
- New optional API endpoints
- Deprecations

**Requires:**
- PR approval
- Updated documentation
- 7-day notice

### PATCH (x.y.Z)
- Bug fixes
- Security patches
- Performance improvements

**Requires:**
- Standard PR approval
- No notice required

## Release Branches

### Main Branch
- Always deployable
- Protected (no direct commits)
- All changes via PR

### Release Branches
```bash
# Create release branch
git checkout -b release/v2.1.0

# Cherry-pick commits
git cherry-pick <commit-sha>

# Finalize
git tag v2.1.0
git push --tags
```

### Hotfix Branches
```bash
# Create from production tag
git checkout -b hotfix/v2.0.1 v2.0.0

# Fix bug
git commit -m "fix: critical bug"

# Tag and deploy
git tag v2.0.1
git push --tags
```

## Rollback Strategy

### Automatic Rollback

Triggered by:
- Error rate > 5%
- Response time > 2x baseline
- Failed health checks

```bash
# Rollback deployment
kubectl rollout undo deployment/billing -n production

# Verify
kubectl rollout status deployment/billing -n production
```

### Manual Rollback

```bash
# Rollback to specific version
kubectl set image deployment/billing billing=ghcr.io/7d-solutions/billing:2.0.1 -n production
```

### Database Rollback

**Forward-only migrations:**
- Write migrations to be reversible
- Never drop columns (mark as deprecated)
- Add columns as nullable first

```sql
-- Migration up (v2.1.0)
ALTER TABLE invoices ADD COLUMN customer_name VARCHAR(255);

-- If rollback needed, column remains but is unused
-- Remove in future MAJOR version
```

## Release Notes

### Format

```markdown
# Billing v2.1.0

**Release Date:** 2026-02-11

## Highlights
- Recurring billing support
- Tax calculation improvements
- Performance optimizations

## Features
- **Recurring Invoices** ([#123](link))
  Added support for monthly/yearly recurring invoices
  
- **Custom Tax Rules** ([#145](link))
  Tax rules now configurable per customer

## Fixes
- Fixed tax calculation rounding error ([#178](link))
- Fixed invoice PDF generation for non-ASCII characters ([#182](link))

## Performance
- Reduced invoice generation time by 40%
- Optimized database queries for customer search

## Breaking Changes
None

## Deprecations
- `GET /api/v1/invoices` - Use `GET /api/v2/invoices` instead
  Will be removed in v3.0.0 (June 2026)

## Migration Guide
No migration required.

## Dependencies
- Requires platform v3.1.0+
- Compatible with customer module v1.5.0+

## Known Issues
None
```

## Release Calendar

### 2026 Q1 Schedule

| Date | Event | Version |
|------|-------|---------|
| Feb 11 | Platform Release | platform/v3.1.0 |
| Feb 18 | Module Release | billing/v2.1.0 |
| Feb 25 | Product Release | fireproof-erp/v5.2.0 |
| Mar 4 | Platform Release | platform/v3.2.0 |
| Mar 11 | Module Release | inventory/v1.9.0 |
| Mar 18 | Product Release | fireproof-erp/v5.3.0 |

## Release Metrics

Track and report:

**Lead Time:**
- Commit to deploy: < 24 hours
- Feature to production: < 2 weeks

**Deployment Frequency:**
- Target: Daily (for PATCH)
- Target: Biweekly (for MINOR)

**Change Failure Rate:**
- Target: < 5%

**Mean Time to Recovery (MTTR):**
- Target: < 1 hour

## Communication

### Pre-Release
**7 days before:**
- Post in #engineering
- Email to stakeholders
- Update release calendar

**2 days before (code freeze):**
- Reminder in #engineering
- Lock release branch

### Release Day
**During release:**
- Status updates in #releases
- Monitor dashboards

**Post-release:**
- Success announcement
- Release notes published
- Customer notification (if applicable)

## Approval Matrix

### PATCH Release
- [ ] 1 approval from module owner
- [ ] CI passing
- [ ] Smoke tests passing

### MINOR Release
- [ ] 2 approvals from module team
- [ ] Tech lead sign-off
- [ ] Full test suite passing
- [ ] Contract tests passing

### MAJOR Release
- [ ] Architecture review
- [ ] Tech lead approval
- [ ] Product team approval
- [ ] 30-day notice period completed
- [ ] Migration guide reviewed

## See Also

- [Versioning Standard](../architecture/VERSIONING-STANDARD.md) - SemVer details
- [Change Control](CHANGE-CONTROL.md) - PR and review process
- [Code Ownership](CODE-OWNERSHIP.md) - Release responsibilities
