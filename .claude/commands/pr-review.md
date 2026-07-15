---
allowed-tools: Agent, Read, Grep, Glob, Bash(gh pr view:*), Bash(gh pr diff:*), Bash(gh pr review:*), mcp__github_inline_comment__create_inline_comment
description: Review a pull request in one pass — framer spec/doc consistency plus general correctness, Rust quality, and test coverage — and submit an approve / request-changes verdict
---

Review the given pull request in a **single pass** that fans out to framer's four
review subagents, validates their findings, and posts one consolidated, high-signal
set of comments. This replaces running separate review tools.

**Arguments: `$ARGUMENTS`** — parse the first token as the PR to review (a URL or a
number; called `<PR>` below). Treat `--comment` anywhere in the arguments as the
write-enable flag. CI also supplies `--metadata <path>` and `--diff <path>` pointing
to one immutable context snapshot. When both paths are present, read them directly
and do not re-fetch PR metadata or the diff. Without `--comment`, review and print
findings only — make no GitHub writes, including no formal review verdict (safe to
run locally).

Follow these steps precisely:

1. **Skip gate.** Read the supplied metadata file, or run one standalone
   `gh pr view <PR>` call when no prepared context was supplied. Never pipe, redirect,
   or chain that command. Stop, doing nothing, ONLY if the PR is closed, or it is a
   draft and you were not explicitly asked to review it, or the change is *both*
   trivial in shape *and* introduces nothing new (a pure
   dependency bump, a pure formatting pass, or a comment-only edit). **Never skip on
   surface shape alone.** A diff is NOT trivial — and MUST be reviewed — if it
   introduces or changes any of: a public API or re-export, an on-disk/serialization
   format or schema, a new module or file format, determinism/canonicalization
   behavior, or error handling — *even if most of its lines are relocated, look
   mechanical, or just move code between files.* Relocating code into a data file,
   raising visibility (`fn` → `pub(crate)`), or adding docs does not make a change
   trivial when it stands up a new format or public surface. When unsure, do NOT skip.
   Otherwise proceed — re-review on every push so the verdict reflects the current head
   and supersedes any earlier verdict. Still review PRs authored by Claude.

2. **Gather context once.** Use the supplied metadata and diff files. For a local
   dry run without prepared paths, make exactly one standalone `gh pr diff <PR>` and
   one standalone `gh pr view <PR>` call; do not pipe, redirect, or chain shell
   commands. Give every reviewer the same two file paths (or the same captured
   contents) so they never re-fetch the PR. The diff is authoritative; the checkout
   may be the PR merge ref in automatic CI or trusted `main` in an on-demand run.
   The author's stated intent matters (a change framed as a refactor isn't held to
   the new-feature spec rule, and needs no new tests).

3. **Run all four reviewers to completion in the foreground.** CI disables
   background tasks mechanically; do not request background execution. Sequential
   execution is acceptable and preferred over an incomplete parallel fan-out. Give
   each reviewer the shared metadata and diff paths, and instruct each to return only
   noteworthy, high-confidence findings (each with file:line, a one-line description,
   the reason, a concrete fix, and a confidence):
   - **spec-consistency-reviewer** — consistency with docs/specs, the `.framer`
     contract, and the architecture invariants (the framer-specific lens).
   - **correctness-reviewer** — real logic bugs, panics, and error-handling defects.
   - **rust-quality-reviewer** — Rust safety/idiom/API and hot-path performance.
   - **test-coverage-reviewer** — behavior changes missing adequate tests.

   Do not consolidate or finish while any reviewer is still running. If a reviewer
   cannot be dispatched or returns nothing usable, do NOT abandon the review — perform
   that dimension yourself directly from the shared diff before proceeding. The four
   completed dimensions are required; subagent dispatch itself is not.

4. **Validate directly.** The parent validates every candidate against the shared
   diff and relevant repository files: the issue must be real, in scope for changed
   lines, and not a false positive or exempt refactor/trivial edit. Do not spawn a
   second wave of validation subagents. Drop anything not validated, and drop every
   `medium`- or lower-confidence finding. **False positives erode trust — when in
   doubt, drop it.**

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

7. **Report and stamp.** Prepare a terminal summary split into Must-fix and Advisory
   (or "No high-signal issues found across spec, correctness, Rust quality, and
   tests"). A progress message or prepared summary is never a valid final response
   while reviewers, comments, or the formal verdict remain outstanding.

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

   When `--comment` is provided and the skip gate did not fire, you **MUST** submit
   exactly one verdict before finishing — even if zero inline comments survive, submit
   `--approve` with a body noting "No high-signal issues found." Finishing with neither
   inline comments nor a verdict is a failure of this command, not a valid outcome.

8. **Completion checklist.** Before producing the final response, verify all of the
   following from actual tool results:
   - all four reviewer dimensions returned usable results, or the parent completed
     the missing dimension itself;
   - every surviving finding was directly validated and duplicates were consolidated;
   - every attempted inline comment succeeded;
   - with `--comment`, exactly one `gh pr review` command succeeded.

   If any item is incomplete, continue working or report a hard failure. Never finish
   with "reviewers are running," "I'll wait," or another progress-only message. The
   workflow independently verifies the fresh formal verdict and fails closed if this
   contract is not met.

Keep every comment concise and grounded. If you cannot justify a finding from the
diff (or quote the spec/invariant it contradicts), do not post it — and never let an
advisory nit drive a request-changes verdict.
