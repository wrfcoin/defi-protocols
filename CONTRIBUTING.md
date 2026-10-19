# Contributing

Read the [README](README.md) for this repository's purpose and technical details.
Follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Build and test

Use Rust 1.88.0, matching CI, and Python 3:

```sh
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo fmt --all -- --check
python3 scripts/check_mission_guardrails.py --repo-root .
python3 -m unittest scripts/test_check_mission_guardrails.py
```

For Solidity changes, follow the README: from `contracts/`, run `npm install`, `npx hardhat compile`, and `npx hardhat test`. The testnet surface remains simulator-only.

## Branch and pull request flow

Fork the repository and create a focused branch from current `main`.
Add tests for behavior changes and run the checks above before submitting a pull request.
Describe the change, its motivation, and validation results; use `Refs #N` to link the issue.
Keep unrelated changes separate and wait for maintainer review before merging.
Report vulnerabilities through [SECURITY.md](SECURITY.md).
