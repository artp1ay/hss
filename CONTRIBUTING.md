# Contributing to hss

Thank you for your interest in contributing to `hss`!

## Code of Conduct

Please be respectful, constructive, and helpful when interacting with the community.

## Development Workflow

1. Fork the repository and clone your fork.
2. Create a feature branch:
   ```bash
   git checkout -b feat/your-feature
   ```
3. Ensure formatting and clippy rules pass:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
   ```
4. Run tests:
   ```bash
   RUST_TEST_THREADS=1 cargo test --workspace --all-targets --all-features --locked
   ```
5. Commit your changes with clear, concise messages.
6. Open a Pull Request targeting `main` (or `dev`).

## Pull Request Guidelines

- Keep changes minimal, focused, and well-tested.
- Maintain existing coding patterns and architectural principles.
- Avoid introducing unnecessary external dependencies.
