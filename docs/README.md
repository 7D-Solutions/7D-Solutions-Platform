# AgentCore Documentation Index

Quick navigation for agents and operators.

## Root-Level Docs

| File | Audience | Description |
|------|----------|-------------|
| [README.md](../README.md) | Everyone | Project overview, core components, getting started |
| [AGENTS.md](../AGENTS.md) | Agents | Operating instructions — beads workflow, search, mail, retros |
| [AGENT_MAIL.md](../AGENT_MAIL.md) | Agents | Mail system quick-start and notification handling |
| [CLAUDE.md](../CLAUDE.md) | Claude agents | Auto-loaded instructions — NTM, mail handling, pointers to AGENTS.md |

## Guides

| File | Description |
|------|-------------|
| [installation.md](installation.md) | Setup guide — prerequisites, clone, install, first session |
| [feature-reference.md](feature-reference.md) | Comprehensive reference for all AgentCore systems (beads, mail, flywheel, CASS, NTM, hooks, retros) |
| [architecture.md](architecture.md) | System architecture — flywheel pattern, component layers, data flow, scaling, security model |
| [COMPATIBILITY-MATRIX.md](COMPATIBILITY-MATRIX.md) | Known-good module version combinations per release train |
| [supervisord-usage.md](supervisord-usage.md) | Process management — starting, stopping, and monitoring mail monitors and beadmonitor |

## Operational Reference

| File | Description |
|------|-------------|
| [../scripts/README.md](../scripts/README.md) | Index of all 52 operational scripts — grouped by function |
| [../agentcore/verify/](../agentcore/verify/) | Verification scripts — phase gate tests, mail roundtrip, registry ops |

## System Schemas

| File | Description |
|------|-------------|
| [rie/close-event-schema.md](rie/close-event-schema.md) | RIE close-event schema — structure of bead close events for retrospectives |

## Where to Start

- **New agent?** Read [AGENTS.md](../AGENTS.md) — it has everything you need for your first session.
- **Setting up a new project?** Read [installation.md](installation.md).
- **Looking up a specific feature?** Read [feature-reference.md](feature-reference.md).
- **Orchestrator?** Start with [AGENTS.md](../AGENTS.md) → Orchestrator section, then [feature-reference.md](feature-reference.md) for the full system map.
