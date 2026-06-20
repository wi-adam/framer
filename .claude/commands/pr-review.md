---
allowed-tools: Bash(gh pr view:*), Bash(gh pr diff:*), Bash(gh pr comment:*), Bash(gh pr list:*), mcp__github_inline_comment__create_inline_comment
description: Review a pull request in one pass — framer spec/doc consistency plus general correctness, Rust quality, and test coverage
---

Review the given pull request in a **single pass** that fans out to framer's four
review subagents, validates their findings, and posts one consolidated, high-signal
set of comments. This replaces running separate review tools.

The argument is a PR (URL or number), optionally followed by `--comment`. Without
`--comment`, review and print findings only — post nothing (safe to run locally).

Follow these steps precisely:

1. **Skip gate.** Look at the PR (`gh pr view <PR>` and its comments). Stop, posting
   nothing, if: the PR is closed or a draft; the change is obviously trivial and
   correct (a dependency bump, a pure formatting or mechanical edit); or framer has
   already posted a review on this PR (`gh pr view <PR> --comments` — don't
   duplicate). Still review PRs authored by Claude.

2. **Gather context once.** Get the diff (`gh pr diff <PR>`) and the PR title + body
   (`gh pr view <PR>`). You will pass this same context to every reviewer so they
   don't each re-fetch it; the author's stated intent matters (a change framed as a
   refactor isn't held to the new-feature spec rule, and needs no new tests).

3. **Fan out — launch these four subagents in parallel**, giving each the diff and
   the PR title/description, and instructing each to return only noteworthy,
   high-confidence findings (each with file:line, a one-line description, the reason,
   a concrete fix, and a confidence):
   - **spec-consistency-reviewer** — consistency with docs/specs, the `.framer`
     contract, and the architecture invariants (the framer-specific lens).
   - **correctness-reviewer** — real logic bugs, panics, and error-handling defects.
   - **rust-quality-reviewer** — Rust safety/idiom/API and hot-path performance.
   - **test-coverage-reviewer** — behavior changes missing adequate tests.

4. **Validate.** For each candidate finding, launch a subagent to confirm it with
   high confidence before it can be posted — that the issue is real, in scope for the
   changed lines, and not a false positive or an exempt refactor/trivial edit. Drop
   anything not validated, and drop every `medium`- or lower-confidence finding.
   **False positives erode trust — when in doubt, drop it.**

5. **Consolidate.** Merge findings across the four dimensions: collapse duplicates
   (e.g. a determinism issue flagged by both the spec and Rust reviewers) into one
   comment under the most fitting dimension. Keep at most one comment per unique
   issue.

6. **Report.** Print a summary to the terminal grouped by dimension, or state "No
   high-signal issues found across spec, correctness, Rust quality, and tests."
   - If `--comment` was **not** provided, stop here. Post nothing.
   - If `--comment` was provided and **no** issues survived, post one short top-level
     summary with `gh pr comment` and stop.
   - If `--comment` was provided and issues survived, post **one** inline comment per
     unique issue with `mcp__github_inline_comment__create_inline_comment`
     (`confirmed: true`), anchored at the relevant line. Prefix each with its
     dimension — `[spec]`, `[bug]`, `[rust]`, or `[tests]` — name the rule or
     spec line for spec findings, explain the issue, and give the concrete fix.
     Include a committable suggestion block only for a small fix that fully resolves
     the issue. Never post duplicate comments.

Keep every comment concise and grounded. If you cannot justify a finding from the
diff (or quote the spec/invariant it contradicts), do not post it.
