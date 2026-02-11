# CI Contract Test Job

## Purpose
Ensure that contracts are not only syntactically valid, but also:
- validated against example payloads
- minimally checked for required API surface
- protected against accidental breaking changes

## What it runs
- JSON schema validation of `contracts/events/examples/*.json`
- YAML parsing and path presence checks for `contracts/*/*.yaml`

## Failure modes
- Missing example for a schema
- Example does not validate against schema
- OpenAPI spec invalid YAML
- OpenAPI missing required paths

## Output requirement
CI must print:
- which schema/example failed
- which OpenAPI file/path failed
