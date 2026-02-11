# Contract Governance

- OpenAPI YAML files in /contracts are the authoritative public surface.
- Rust code may generate candidate specs.
- Generated output must be reviewed and committed manually.
- CI will eventually validate that generated spec matches committed spec.
- Breaking contract change requires:
  - MAJOR version bump (module)
  - CHANGELOG update
  - Contract version update
