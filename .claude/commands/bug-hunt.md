Run a three-pass adversarial bug hunt on the files specified by the user: $ARGUMENTS

If no files are specified, use `git diff --name-only HEAD~1` to get recently changed files.

## Pass 1: Hunter (aggressive)

Adopt the mindset of an aggressive bug hunter. Your goal is to find EVERY possible bug, no matter how unlikely. For each file:

- Read the file completely
- Look for: null/undefined risks, race conditions, off-by-one errors, resource leaks, unhandled edge cases, security issues, logic errors, type mismatches, missing error handling, silent failures, incorrect assumptions about input
- Be paranoid. Flag anything that COULD go wrong, even if unlikely
- For each finding, output:

```
[HUNTER-NNN] severity:(critical|high|medium|low) file:line
  Description: What the bug is
  Impact: What goes wrong if triggered
  Evidence: The specific code pattern that's problematic
```

## Pass 2: Skeptic (adversarial)

Now switch mindset completely. You are a skeptic who HATES false positives. Review every Hunter finding and challenge it:

- Is this actually reachable in practice?
- Does the surrounding code already handle this case?
- Is the framework/language/runtime providing implicit protection?
- Would this ever trigger given realistic inputs?

IMPORTANT: You have a 2x penalty for wrongly dismissing a real bug. Be skeptical but fair. If there's genuine risk, even if unlikely, uphold the finding.

For each Hunter finding, output:

```
[SKEPTIC on HUNTER-NNN] verdict:(upheld|disputed|dismissed)
  Reasoning: Why you upheld or challenged this finding
  If disputed: What specific evidence suggests it's not a real bug
```

## Pass 3: Referee (final verdict)

Now act as a neutral referee. For any DISPUTED findings (not upheld or dismissed), make a final call:

- Weigh the Hunter's evidence against the Skeptic's challenge
- Consider: Is the code defensively written? Is the risk worth flagging?
- Err on the side of flagging if there's genuine ambiguity

```
[REFEREE on HUNTER-NNN] final:(confirmed-bug|false-positive|worth-investigating)
  Final reasoning: One sentence explanation
```

## Summary

After all three passes, output a final summary table:

| # | Severity | File:Line | Description | Verdict |
|---|----------|-----------|-------------|---------|
| 1 | critical | ... | ... | confirmed-bug |

Only include confirmed bugs and worth-investigating items. Order by severity (critical first).

End with a count: "X confirmed bugs, Y worth investigating, Z false positives filtered out of N total findings"
