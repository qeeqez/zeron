# Agent Guide: rixlcode

## Verification Commands

Run before every commit and PR:
- `mise run verify` – full build + test gate (`cargo nextest run --locked --no-tests=pass`)
- `mise run lint` – rustfmt + clippy check
- `mise run lint:sloc` – enforces max 250 SLOC per file (comments and blanks excluded) via `sloc-guard`
- `cargo clippy --all-targets -- -D warnings` – strict clippy lint
- `cargo fmt --check` / `cargo fmt` – formatting
- `cargo nextest run --locked --no-tests=pass` – parallel unit and integration tests; the only test runner (never `cargo test`)

## Lint Policy

- **No per-site clippy suppressions.** `#[allow(clippy::…)]`, `#[expect(clippy::…)]`, `#![allow(clippy::…)]`, and `#![expect(clippy::…)]` are prohibited at item and crate level. When a lint fires, fix the code: bundle parameters into a params struct, extract helpers to reduce nesting, introduce a type alias, or split the function.
- **Policy lives in `clippy.toml`, not in attributes.** If a lint is genuinely wrong for this codebase project-wide, adjust or remove the threshold in `clippy.toml` with a written justification in the PR — never suppress at the call site.
- `cargo clippy --locked --all-targets -- -D warnings` must stay green.

## Repository & Git Conventions

- **File Size Limit**: Max 250 SLOC per file (excluding blanks/comments), strictly enforced by `mise run lint:sloc` via `sloc-guard`. Split large modules into subdirectories with `types.rs`, `runner.rs`, `mod.rs`.
- **Git Commit Attribution**: Exactly one `Co-authored-by: Rixl <agent@rixl.com>` trailer on every commit. No other attribution tags.
- **PR Merge Policy**: **Rebase merge only** (`gh pr merge --rebase`) — keeps history linear. **NEVER squash merge, NEVER merge-commit.**
- **Subagent Workflows**: Always run subagents in isolated Copy-on-Write clones under `/tmp/rixl-rixlcode/<name>` on their own `rixl/<lane>` branch. Do NOT set a shared `CARGO_TARGET_DIR` (all lanes would serialize on one target lock); instead `cargo build`/`nextest` the main checkout periodically so each clone inherits a warm `target/` via CoW and builds in its own directory.
