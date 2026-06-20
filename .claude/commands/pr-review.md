---
allowed-tools: Bash(gh pr view:*), Bash(gh pr diff:*), Bash(gh pr comment:*), Bash(gh pr review:*), Bash(gh pr list:*), mcp__github_inline_comment__create_inline_comment
description: Review a pull request in one pass — framer spec/doc consistency plus general correctness, Rust quality, and test coverage — and submit an approve / request-changes verdict
---

Review the given pull request in a **single pass** that fans out to framer's four
review subagents, validates their findings, and posts one consolidated, high-signal
set of comments. This replaces running separate review tools.

The argument is a PR (URL or number), optionally followed by `--comment`. Without
`--comment`, review and print findings only — make no GitHub writes, including no
formal review verdict (safe to run locally).

Follow these steps precisely:

1. **Skip gate.** Look at the PR (`gh pr view <PR>`). Stop, doing nothing, if the PR
   is closed or a draft, or the change is obviously trivial and correct (a dependency
   bump, a pure formatting or mechanical edit). Otherwise proceed — re-review on every
   push so the verdict reflects the current head and supersedes any earlier verdict.
   Still review PRs authored by Claude.

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

6. **Classify severity.** Split the surviving findings into **blocking** (must-fix:
   correctness/security defects, an invariant or spec violation, a missing required
   spec for a product-visible/schema change, new behavior left untested, or a
   swallowed error) and **advisory** (non-blocking nits: idiom, type-design/API
   polish, simplification, spec-hygiene/traceability). The verdict is **request
   changes** if at least one blocking finding survived, otherwise **approve** —
   advisory-only PRs are approved with the nits noted, never blocked.

7. **Report and stamp.** Print a terminal summary split into Must-fix and Advisory
   (or "No high-signal issues found across spec, correctness, Rust quality, and
   tests").

   - If `--comment` was **not** provided, stop here — make no GitHub writes. This is
     the local dry-run path.
   - If `--comment` was provided:
     a. Post **one** inline comment per unique finding with
        `mcp__github_inline_comment__create_inline_comment` (`confirmed: true`),
        anchored at the line. Prefix blocking findings with their dimension —
        `[spec]` / `[bug]` / `[rust]` / `[tests]` — and advisory findings with
        `[nit]` so it is unmistakable they are not blockers. Name the rule or spec
        line for spec findings; give the concrete fix; include a committable
        suggestion block only for a small fix that fully resolves the issue. Never
        post a duplicate comment.
     b. Submit one formal review verdict (this is the PR's quality gate that CI reads
        to allow or block merge):
        - if any blocking finding survived →
          `gh pr review <PR> --request-changes --body "<summary>"`
        - otherwise →
          `gh pr review <PR> --approve --body "<summary>"`
        `<summary>` is a short `## Claude review` block stating the verdict, a
        **Must fix** list (the blocking findings, or "none"), and the **Advisory**
        count. Submit exactly one verdict per run.

Keep every comment concise and grounded. If you cannot justify a finding from the
diff (or quote the spec/invariant it contradicts), do not post it — and never let an
advisory nit drive a request-changes verdict.
