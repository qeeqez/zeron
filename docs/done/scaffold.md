# Scaffold & tooling — DONE

- Rust-only project, nightly toolchain (`rust-toolchain.toml`), edition 2024
- `mise.toml` tasks: setup / fmt / fmt:files / lint / lint:sloc / build / test (nextest only, `--no-tests=pass`) / verify
- `clippy.toml` strict thresholds; no per-site suppressions (policy in AGENTS.md)
- `rustfmt.toml` style_edition 2024, max_width 140
- `.sloc-guard.toml` 250 SLOC/file
- `lefthook.yml` pre-commit fmt+lint+sloc, pre-push verify
- AGENTS.md: verification commands, lint policy, git conventions (rebase-merge only, conventional commits, Rixl trailer)
- `docs/loop.md` lane mechanics
