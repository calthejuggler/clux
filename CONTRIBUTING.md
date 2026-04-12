# Contributing to clux

Thanks for wanting to help out. Here's what you need to know.

## Getting started

1. Fork the repo and clone it locally
2. Make sure you have Rust installed (stable toolchain)
3. Run `cargo build` to check everything compiles
4. Run `cargo test` to make sure tests pass

If you want to test the tmux integration, you'll need tmux and Claude Code running.

## Making changes

1. Create a branch off `main`
2. Make your changes
3. Run the checks locally before pushing:
   ```sh
   cargo fmt -- --check
   cargo clippy -- -D warnings
   cargo test
   ```
4. Open a pull request against `main`

## Code style

- This project uses strict clippy lints. If clippy complains, fix the code rather than adding `#[allow(...)]` attributes.
- Run `cargo fmt` before committing. The project uses the formatting rules in `rustfmt.toml`.
- Keep code self-documenting. Comments should be a last resort, not a default.

## What to work on

- Check the [open issues](https://github.com/calthejuggler/clux/issues) for things that need attention
- If you have an idea for something new, open an issue first so we can talk about it before you write a bunch of code

## Bug reports

If you find a bug, open an issue with:

- What you expected to happen
- What actually happened
- Steps to reproduce it
- Your OS and Rust version

## Releases

Releases are handled automatically via release-plz. You don't need to worry about versioning or changelogs -- just write good commit messages.
