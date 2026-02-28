# AGENTS.md

## Project Overview
`chrome-cdp` is a Rust library designed to interact with Chrome via the DevTools Protocol (CDP).

## Development Rules
- **Language**: All code, comments, documentation, commit messages, and PR descriptions MUST be in English.
- **Commits**: Use [Conventional Commits](https://www.conventionalcommits.org/).
- **Testing**: Use `cargo test` to verify changes.
- **Entry point**: The primary logic resides in `src/lib.rs`.
- **Pre-commit hooks**: NEVER use `git commit --no-verify` to bypass pre-commit hooks. Fix issues instead.
