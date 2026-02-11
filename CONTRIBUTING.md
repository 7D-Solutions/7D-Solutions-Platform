# Contributing to 7D Solutions Platform

Thank you for your interest in contributing to the 7D Solutions Platform! This document provides guidelines for contributing code, documentation, and other improvements.

## Code of Conduct

Be respectful, constructive, and professional in all interactions.

## Getting Started

### Prerequisites

- Node.js 20+
- Docker & Docker Compose
- pnpm 9+
- Git

### Setup

```bash
# Clone repository
git clone https://github.com/7d-solutions/platform.git
cd platform

# Install dependencies
pnpm install

# Start development environment
docker compose -f infra/docker/docker-compose.dev.yml up -d

# Run tests
pnpm test
```

## Development Workflow

### 1. Create a Branch

```bash
# Feature
git checkout -b feature/module-name-feature-description

# Bug fix
git checkout -b fix/module-name-bug-description

# Breaking change
git checkout -b breaking/module-name-change-description
```

### 2. Make Changes

- Follow the [Module Standard](docs/architecture/MODULE-STANDARD.md)
- Follow the [Layering Rules](docs/architecture/LAYERING-RULES.md)
- Write tests for all changes
- Update documentation

### 3. Test Your Changes

```bash
# Run all tests
pnpm test

# Run tests for specific module
pnpm test --filter @7d-platform/billing

# Run contract tests
pnpm test:contracts

# Lint code
pnpm lint

# Format code
pnpm format
```

### 4. Commit Your Changes

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```bash
# Feature
git commit -m "feat(billing): add recurring invoice support"

# Bug fix
git commit -m "fix(inventory): correct stock calculation"

# Breaking change
git commit -m "feat(billing)!: remove deprecated v1 API

BREAKING CHANGE: Removed /api/v1/invoices endpoint.
Use /api/v2/invoices instead."
```

### 5. Push and Create PR

```bash
git push origin feature/billing-recurring-invoices
```

Then create a Pull Request on GitHub.

## Pull Request Guidelines

### PR Title

Use conventional commit format:
- `feat(module): description`
- `fix(module): description`
- `docs(module): description`

### PR Description

Include:
- **What** changed
- **Why** it changed
- **How** to test it
- **Breaking changes** (if any)
- **Related issues** (closes #123)

### Checklist

Before submitting PR:

- [ ] Tests pass locally
- [ ] Code follows style guidelines
- [ ] Documentation updated
- [ ] Contract tests pass (if API changes)
- [ ] No new warnings
- [ ] Self-review completed

## Architecture Guidelines

### Three-Tier Architecture

```
products/ → modules/ → platform/
```

- **Platform:** Core runtime (identity, events, orchestration)
- **Modules:** Business logic (billing, inventory, QMS)
- **Products:** Composed applications (Fireproof ERP)

See [Monorepo Standard](docs/architecture/MONOREPO-STANDARD.md) for details.

### Module Structure

```
modules/{module-name}/
├── domain/       # Pure business logic
├── repos/        # Data access
├── services/     # Application services
├── routes/       # HTTP handlers
└── tests/        # Tests
```

See [Module Standard](docs/architecture/MODULE-STANDARD.md) for details.

### Communication Between Modules

- ✅ REST API calls (via contracts)
- ✅ Event bus (via contracts)
- ❌ Direct source imports

See [Contract Standard](docs/architecture/CONTRACT-STANDARD.md) for details.

## Coding Standards

### TypeScript

- Use strict TypeScript (`strict: true`)
- Prefer interfaces over types
- Use meaningful variable names
- Avoid `any` type

### Testing

- Unit tests for domain logic (90%+ coverage)
- Integration tests for services (80%+ coverage)
- Contract tests for APIs
- E2E tests for critical flows

### Documentation

- Document public APIs
- Add JSDoc comments for complex functions
- Update README when adding features
- Add ADRs for architectural decisions

## Review Process

### Approval Requirements

- **Standard changes:** 1 approval from module owner
- **Breaking changes:** 2+ approvals + tech lead sign-off
- **Platform changes:** 2 approvals from platform team

### Review Checklist

Reviewers check for:
- [ ] Correct logic
- [ ] Follows architecture rules
- [ ] No security issues
- [ ] Good test coverage
- [ ] Clear documentation

See [Change Control](docs/governance/CHANGE-CONTROL.md) for details.

## Versioning

We use [Semantic Versioning](https://semver.org/):

- **MAJOR:** Breaking changes
- **MINOR:** New features (backward compatible)
- **PATCH:** Bug fixes

Each module is versioned independently.

See [Versioning Standard](docs/architecture/VERSIONING-STANDARD.md) for details.

## Release Process

See [Release Policy](docs/governance/RELEASE-POLICY.md) for the complete release process.

## Questions?

- Open an issue for bugs or feature requests
- Ask in #engineering Slack channel
- Email: engineering@7dsolutions.com

## License

By contributing, you agree that your contributions will be licensed under the same terms as the project.
