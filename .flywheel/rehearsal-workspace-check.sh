#!/usr/bin/env bash
# Rehearsal Layer-1 workspace sanity check.
#
# Invoked by br-rehearse.sh (AgentCore) after bead impl+verify pass, before the
# rehearsal pass marker is written. Non-zero exit fails rehearsal.
#
# Purpose: catch cross-crate consumer drift that single-crate verify misses.
# bd-44hil (April 2026) regenerated client crates to drop the VerifiedClaims
# arg but left 5 consumer call sites broken across 4 crates. bd-44hil's Verify
# block only ran fmt --check and client-codegen, so rehearsal passed. The
# workspace build failed for 24+ hours in cross-watcher, which silently held
# all subsequent commits off the running containers. See bd-ny9v9 for the
# incident fix.
#
# Inert until AgentCore br-rehearse.sh wires the hook call. Safe to land now.

set -euo pipefail

exec ./scripts/cargo-slot.sh build --workspace
