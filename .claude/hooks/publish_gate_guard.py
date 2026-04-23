#!/usr/bin/env python3
"""
Publish gate guard: blocks any `br create` or `br update` that sets status to
a non-draft "ready for work" value, forcing agents through br-publish.sh
(draft → open) and bv-claim.sh (open → in_progress).

Bypass: trusted scripts (br-publish.sh, bv-claim.sh) prepend
BR_PUBLISH_GATE_BYPASS=1 to their own invocations. This env assignment must
appear on the same command line as the br invocation to be honored.

Uses shlex tokenisation so the phrase "--status open" inside a quoted
description argument does not trigger a false positive.
"""
import json
import shlex
import sys

FORBIDDEN_TARGETS = {'open', 'in_progress', 'ready', 'blocked'}
SHELL_SEPARATORS = {';', '&&', '||', '|', '&'}
BYPASS_TOKEN = 'BR_PUBLISH_GATE_BYPASS=1'  # nosec B105 — public escape-hatch marker, not a secret


def _segments(command: str):
    """Split the command into shell segments at top-level separators.
    Returns list of token lists, or None if unparseable.

    Uses shlex in punctuation-tokenising mode so shell operators (``;``,
    ``&&``, ``||``, ``|``, ``&``) become their own tokens even when not
    space-separated (e.g. ``open; br``). Without this, ``open;`` would be
    a single token and the scanner would miss the following ``br``.
    """
    try:
        lex = shlex.shlex(command, posix=True, punctuation_chars=True)
        lex.whitespace_split = True
        tokens = list(lex)
    except ValueError:
        return None
    segs = []
    cur = []
    for tok in tokens:
        if tok in SHELL_SEPARATORS:
            if cur:
                segs.append(cur)
                cur = []
        else:
            cur.append(tok)
    if cur:
        segs.append(cur)
    return segs


def scan(command: str):
    """Return (kind, target) for the first blocked transition, else None.

    Each shell segment is evaluated independently. The bypass token must
    appear before the `br` token in the SAME segment to allow a forbidden
    transition; bypass in a sibling segment does not carry over.
    """
    segs = _segments(command)
    if segs is None:
        return None
    for seg in segs:
        br_idx = None
        has_bypass = False
        for k, tok in enumerate(seg):
            if tok == 'br':
                br_idx = k
                break
            if tok == BYPASS_TOKEN:
                has_bypass = True
        if br_idx is None or has_bypass:
            continue
        if br_idx + 1 >= len(seg):
            continue
        subcmd = seg[br_idx + 1]
        if subcmd not in ('create', 'update'):
            continue
        j = br_idx + 2
        while j < len(seg):
            tok = seg[j]
            target = None
            if tok == '--status' and j + 1 < len(seg):
                target = seg[j + 1]
                j += 1
            elif tok.startswith('--status='):
                target = tok.split('=', 1)[1]
            elif tok == '-s' and j + 1 < len(seg):
                target = seg[j + 1]
                j += 1
            elif tok.startswith('-s='):
                target = tok.split('=', 1)[1]
            if target is not None and target in FORBIDDEN_TARGETS:
                return (subcmd, target)
            j += 1
    return None


def main():
    try:
        data = json.load(sys.stdin)  # ubs:ignore py.json-load-no-try — already in try/except
    except (json.JSONDecodeError, ValueError):
        sys.exit(0)
    if not isinstance(data, dict) or data.get('tool_name') != 'Bash':
        sys.exit(0)
    tool_input = data.get('tool_input') or {}
    command = tool_input.get('command') if isinstance(tool_input, dict) else None
    if not isinstance(command, str):
        sys.exit(0)

    # Bypass is evaluated per-segment inside scan(); no global short-circuit.
    hit = scan(command)
    if not hit:
        sys.exit(0)
    kind, target = hit
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": (
                f"PUBLISH GATE: `br {kind} ... --status {target}` is blocked.\n\n"
                f"Draft → {target} transitions must go through the rehearsal flow.\n\n"
                "To publish a draft bead to the worker pool:\n"
                "  flywheel_tools/scripts/beads/br-publish.sh <bead-id>\n"
                "  (runs br-rehearse on the bead before promoting draft → open)\n\n"
                "To claim a ready (open) bead as a worker:\n"
                "  flywheel_tools/scripts/beads/bv-claim.sh\n\n"
                "If rehearsal rejects, fix the bead description and retry br-publish.sh."
            ),
        }
    }))
    sys.exit(0)


if __name__ == '__main__':
    main()
