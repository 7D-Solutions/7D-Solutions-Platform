# Release Policy

The 7D Solutions Platform follows a regular release cadence with clear processes for scheduled releases, hotfixes, and emergency deployments.

## Release Cadence

### Modules
- **Frequency:** Biweekly (every 2 weeks)
- **Day:** Tuesday, 10 AM PT
- **Contents:** Features, bug fixes, improvements

### Platform
- **Frequency:** Monthly
- **Day:** First Tuesday of month
- **Contents:** Platform updates, security patches

### Products
- **Frequency:** Monthly
- **Day:** Second Tuesday of month
- **Contents:** Module updates, new features

## Release Types

### Scheduled Release
Regular biweekly/monthly releases following the standard process.

### Hotfix Release
Urgent fixes deployed on-demand (PATCH version bump).

**When:**
- Critical bugs
- Security vulnerabilities
- Data integrity issues

**Process:**
- Create hotfix branch from production tag
- Fix issue
- Deploy immediately
- Follow up with post-mortem

### Emergency Release
Production-down situations requiring immediate deployment.

**When:**
- System down
- Security breach
- Data loss risk

**Process:**
- Bypass normal approvals
- Deploy immediately
- Post-mortem required within 24 hours

## Release Process

### 1. Pre-Release (7 days before)
- [ ] Announce in #engineering
- [ ] Update release calendar
- [ ] Notify stakeholders

### 2. Code Freeze (2 days before)
- [ ] Lock release branch
- [ ] Only critical fixes allowed
- [ ] Final testing begins

### 3. Release Day
- [ ] Deploy to staging
- [ ] Run smoke tests
- [ ] Deploy to production
- [ ] Monitor metrics
- [ ] Announce completion

### 4. Post-Release
- [ ] Verify deployment health
- [ ] Update documentation
- [ ] Publish release notes
- [ ] Customer notification (if needed)

## Version Bumping

```bash
# PATCH - Bug fixes
npm version patch

# MINOR - New features
npm version minor

# MAJOR - Breaking changes
npm version major
```

## Git Tags

Format: `{module}-v{version}`

```bash
git tag billing-v2.3.1
git push --tags
```

## Docker Images

```bash
# Build
docker build -t ghcr.io/7d-solutions/billing:2.3.1 .

# Tag
docker tag ghcr.io/7d-solutions/billing:2.3.1 ghcr.io/7d-solutions/billing:2

# Push
docker push ghcr.io/7d-solutions/billing:2.3.1
docker push ghcr.io/7d-solutions/billing:2
```

## Deployment

### Staging
```bash
# Deploy to staging
kubectl set image deployment/billing billing=ghcr.io/7d-solutions/billing:2.3.1 -n staging

# Verify
kubectl rollout status deployment/billing -n staging
```

### Production
```bash
# Deploy with rolling update (24 hours after staging)
kubectl set image deployment/billing billing=ghcr.io/7d-solutions/billing:2.3.1 -n production

# Monitor
kubectl rollout status deployment/billing -n production
```

## Rollback

### Automatic Rollback
Triggered when:
- Error rate > 5%
- Response time > 2x baseline
- Failed health checks

### Manual Rollback
```bash
# Rollback to previous version
kubectl rollout undo deployment/billing -n production

# Rollback to specific version
kubectl set image deployment/billing billing=ghcr.io/7d-solutions/billing:2.3.0 -n production
```

## Release Notes Format

```markdown
# Module Name v2.3.1

**Release Date:** 2026-02-11

## Highlights
- Key feature 1
- Key feature 2

## Features
- Feature description ([#123](link))

## Fixes
- Bug fix description ([#456](link))

## Breaking Changes
None / Description of breaking changes

## Migration Guide
Steps to migrate (if breaking changes)

## Dependencies
- Requires platform v3.1.0+
```

## Approval Requirements

### PATCH Release
- [ ] 1 approval from module owner
- [ ] CI passing

### MINOR Release
- [ ] 2 approvals from module team
- [ ] Tech lead sign-off
- [ ] All tests passing

### MAJOR Release
- [ ] Architecture review
- [ ] Tech lead approval
- [ ] Product approval
- [ ] 30-day notice period
- [ ] Migration guide

## Communication

### Pre-Release (7 days)
- Post in #engineering
- Email stakeholders
- Update calendar

### Release Day
- Status updates in #releases
- Success announcement
- Publish release notes

### Post-Release
- Customer notification (if applicable)
- Update documentation

## Metrics

Track and monitor:
- **Lead Time:** Commit to deploy
- **Deployment Frequency:** Releases per week
- **Change Failure Rate:** % of releases requiring rollback
- **MTTR:** Mean time to recovery

**Targets:**
- Lead time: < 24 hours
- Deployment frequency: Daily (PATCH), Biweekly (MINOR)
- Change failure rate: < 5%
- MTTR: < 1 hour

## Release Calendar

View upcoming releases at: https://calendar.7dsolutions.com/releases

## Detailed Documentation

For complete release policies and procedures, see:
- [Release Policy (Detailed)](docs/governance/RELEASE-POLICY.md)
- [Change Control](docs/governance/CHANGE-CONTROL.md)
- [Code Ownership](docs/governance/CODE-OWNERSHIP.md)
- [Versioning Standard](docs/architecture/VERSIONING-STANDARD.md)

## Support

Questions about releases?
- #engineering Slack channel
- Email: releases@7dsolutions.com
- See [CONTRIBUTING.md](CONTRIBUTING.md)
