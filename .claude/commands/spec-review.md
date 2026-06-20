---
allowed-tools: Bash(gh pr view:*), Bash(gh pr diff:*), Bash(gh pr comment:*), Bash(gh pr list:*), mcp__github_inline_comment__create_inline_comment
description: Review a pull request for consistency with framer's specs, docs, and architecture invariants
---

Review the given pull request for **consistency with framer's written intent** —
its durable specs (`docs/specs/`), the `.framer` format contract
(`docs/project-files.md`), and the architecture invariants (`AGENTS.md` /
`docs/architecture.md`). This is a *focused* review: it complements, and does not
duplicate, the general bug/quality review. Do not raise generic bug, style, or
performance issues here — only spec/doc/invariant consistency.

The argument is a PR (URL or number), optionally followed by `--comment`. Without
`--comment`, review and print findings only — post nothing (this makes the command
safe to run locally).

Follow these steps precisely:

1. **Skip gate.** Look at the PR (`gh pr view <PR>` and its comments). Stop, posting
   nothing, if any of these hold:
   - the PR is closed or a draft;
   - the change plainly needs no spec review — a pure bug fix, a no-behavior
     refactor, a dependency bump, or a trivial/mechanical edit;
   - a framer spec-consistency review comment from a previous run already exists on
     this PR (check `gh pr view <PR> --comments`; do not duplicate it).
   Still review PRs authored by Claude.

2. **Gather context.** Get the diff (`gh pr diff <PR>`) and the PR title + body
   (`gh pr view <PR>`). The author's stated intent matters: a change the author
   frames as a refactor should not be held to the new-feature "spec required" rule.

3. **Dispatch the reviewer.** Launch the `spec-consistency-reviewer` subagent, giving
   it the diff and the PR title/description. It will read the relevant specs and
   invariants and return candidate findings, each with a file:line, the quoted rule
   it contradicts, why, a concrete fix, and a confidence.

4. **Validate.** For each candidate finding, launch a subagent to confirm it with
   high confidence before it can be posted: that the cited spec line / invariant /
   contract genuinely applies to these changed files, that the spec really was *not*
   already updated in the diff, and that the change isn't an exempt bug fix or
   no-behavior refactor. Drop anything not validated, and drop every `medium`- or
   lower-confidence finding. **False positives erode trust — when in doubt, drop it.**

5. **Report.** Print a summary to the terminal: list each confirmed finding briefly,
   or state "No spec/doc/invariant inconsistencies found." 

   - If `--comment` was **not** provided, stop here. Post nothing.
   - If `--comment` was provided and **no** issues were confirmed, post a short
     top-level summary with `gh pr comment` (e.g. "Spec-consistency review: no
     inconsistencies with specs, docs, or invariants found.") and stop.
   - If `--comment` was provided and issues were confirmed, post **one** inline
     comment per issue with `mcp__github_inline_comment__create_inline_comment`
     (`confirmed: true`), anchored at the relevant line. Each comment: name the
     spec/invariant (quote the contradicted line), explain the inconsistency, and
     give the concrete fix. For a small, self-contained doc/code fix, include a
     committable suggestion block only if committing it fully resolves the issue.
     Post at most one comment per unique issue; never duplicate.

Keep every comment concise and tied to a specific written rule. If you cannot quote
the spec line, contract, or invariant that a change contradicts, do not post it.
