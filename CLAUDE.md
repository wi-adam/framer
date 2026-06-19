# CLAUDE.md

This repo's contributor & agent contract lives in **[AGENTS.md](AGENTS.md) — read it
first.** It has the repo map, architecture invariants, the spec-driven workflow, and
the verification gates. Everything below is Claude-specific and additive.

## Start here

- [AGENTS.md](AGENTS.md) — the contract (repo map, invariants, how we work, gates).
- [docs/code-map.md](docs/code-map.md) — where things live (modules, types, data-flow).
- [docs/spec-driven-development.md](docs/spec-driven-development.md) — specs vs. plans.

## Claude-specific

- **Visual verification:** GUI tools (computer-use/screenshots) only see *installed*
  app bundles, never a `cargo run`/`target` binary. Use the
  [`install-app`](.claude/skills/install-app) skill to build + install
  `~/Applications/Framer.app`, then drive that.
- **Before committing**, run the gates from [AGENTS.md](AGENTS.md#verification-gates-must-pass-before-commit):
  `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`,
  `cargo test --workspace --all-features --locked`.
- Keep `framer-core`/`framer-solver`/`framer-render` UI-free and the model
  deterministic (integer ticks, ID-sorted canonical `.framer`). See the invariants
  in [AGENTS.md](AGENTS.md#architecture-invariants-do-not-break).
