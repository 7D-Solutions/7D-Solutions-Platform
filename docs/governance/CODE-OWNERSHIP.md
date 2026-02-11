# Code Ownership

**Version:** 1.0  
**Status:** Active  
**Last Updated:** 2026-02-11

## Overview

This document defines ownership, responsibilities, and decision-making authority for code in the 7D Solutions Platform.

## Ownership Model

### Module Owners

Each module has a designated **owner** responsible for:

**Technical Decisions:**
- API design and breaking changes
- Architecture and implementation approach
- Performance and scalability
- Technology choices within the module

**Code Quality:**
- Review and approve PRs
- Maintain test coverage
- Ensure documentation is current
- Address technical debt

**Operational Responsibility:**
- Monitor production health
- Respond to incidents
- Performance optimization
- Coordinate releases

### Platform Owners

Platform components (identity, events, orchestration) have owners responsible for:
- Core platform capabilities
- Cross-cutting concerns
- Infrastructure and operations
- Platform-wide standards

### Product Owners

Products have owners responsible for:
- Feature prioritization
- Module composition
- Customer requirements
- Release planning

## RACI Matrix

### Module Development

| Activity | Module Owner | Platform Team | Other Modules | Product Team |
|----------|-------------|---------------|---------------|--------------|
| Feature design | **R/A** | C | I | C |
| API breaking change | **R/A** | C | C | I |
| Implementation | R | I | I | I |
| Code review | **A** | C | I | I |
| Release decision | **A/R** | C | I | I |
| Production incident | **R** | C | I | A |

**Legend:**
- **R** - Responsible (does the work)
- **A** - Accountable (makes the decision)
- **C** - Consulted (input requested)
- **I** - Informed (kept updated)

### Platform Changes

| Activity | Platform Team | Module Owners | Product Team |
|----------|--------------|---------------|--------------|
| Platform API change | **R/A** | C | I |
| Infrastructure change | **R/A** | I | I |
| Security policy | **R/A** | C | I |
| Deployment process | **R/A** | C | I |

## Decision Rights

### Module Owner Can Decide

**Without approval:**
- Internal refactoring (no API changes)
- Bug fixes
- Performance improvements
- Documentation updates
- Test improvements
- PATCH version releases

**With notification:**
- MINOR version releases (new features)
- New optional API endpoints
- Deprecation notices

**With approval required:**
- MAJOR version releases (breaking changes)
- New required dependencies
- Database schema changes (if shared)
- Changes affecting other modules

### Platform Team Can Decide

**Without approval:**
- Platform-internal changes
- Infrastructure improvements
- Observability enhancements

**With approval required:**
- Breaking changes to platform APIs
- New platform-wide standards
- Changes to deployment process

## CODEOWNERS File

```
# Platform
/platform/                     @platform-team
/platform/identity/            @james @platform-team
/platform/events/              @platform-team
/platform/orchestration/       @platform-team

# Modules
/modules/billing/              @billing-team
/modules/inventory/            @inventory-team
/modules/qms/                  @qms-team
/modules/document-control/     @doccontrol-team

# Contracts
/contracts/api/billing-*       @billing-team
/contracts/api/inventory-*     @inventory-team
/contracts/events/*            @platform-team @module-owners

# Products
/products/fireproof-erp/       @fireproof-team

# Infrastructure
/infra/                        @platform-team @devops
/tools/ci/                     @platform-team

# Documentation
/docs/architecture/            @platform-team
/docs/governance/              @james
```

## Approval Requirements

### Pull Request Approval

**Module PRs:**
- 1 approval from module owner OR
- 2 approvals from module team

**Platform PRs:**
- 2 approvals from platform team
- 1 approval from affected module owners (if breaking change)

**Cross-Module PRs:**
- 1 approval from each affected module owner

**Documentation PRs:**
- 1 approval from any owner

### Breaking Change Approval

**Module breaking changes:**
1. RFC document
2. Review by affected module owners
3. Approval from tech lead
4. Minimum 2-week notice

**Platform breaking changes:**
1. RFC document
2. Review by all module owners
3. Approval from architecture team
4. Minimum 1-month notice

## Escalation Path

```
Module Issue
    ↓
Module Owner
    ↓
Tech Lead
    ↓
Engineering Manager
    ↓
CTO
```

## Handoff Process

When ownership changes:

1. **Knowledge transfer** (2-4 weeks)
   - Architecture walkthrough
   - Code walkthrough
   - Operations runbook review
   - Incident response procedures

2. **Shadowing**
   - New owner shadows current owner
   - PR reviews together
   - Incident response together

3. **Documentation**
   - Update CODEOWNERS
   - Update README
   - Update on-call rotation

4. **Announcement**
   - Notify team
   - Update documentation
   - Transition period (1-2 sprints)

## Ownership Vacancies

If module has no owner:

1. **Short-term** (< 2 weeks)
   - Tech lead covers
   - Critical issues only

2. **Long-term** (> 2 weeks)
   - Assign interim owner
   - Find permanent owner
   - Consider deprecation if no owner available

## Shared Ownership (Discouraged)

Shared ownership leads to:
- Diffused responsibility
- Slower decision-making
- Communication overhead

**If unavoidable:**
- Designate primary owner
- Clear decision-making process
- Explicit on-call rotation

## See Also

- [Change Control](CHANGE-CONTROL.md) - PR and review process
- [Release Policy](RELEASE-POLICY.md) - Release procedures
