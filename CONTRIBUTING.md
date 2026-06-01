[**English**](CONTRIBUTING.md) | [简体中文](CONTRIBUTING-CN.md)

# Contributing to RMQTT

Thank you for considering contributing to RMQTT! This guide outlines the development workflow, code standards, and processes we follow.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Reporting Issues](#reporting-issues)
- [Development Setup](#development-setup)
- [Code Style](#code-style)
- [Testing](#testing)
- [Documentation](#documentation)
- [Pull Request Process](#pull-request-process)
- [License](#license)

## Code of Conduct

This project is committed to providing a welcoming and inclusive experience for everyone. We expect all contributors to:

- Be respectful and considerate
- Accept constructive criticism gracefully
- Focus on what is best for the community
- Show empathy towards other community members

## Reporting Issues

Before submitting an issue:

1. **Search existing issues** — check if the problem has already been reported
2. **Use a clear, descriptive title** — summarize the problem in one sentence
3. **Include reproduction steps** — the minimum code/config required to reproduce
4. **Specify your environment** — OS, Rust version (`rustc --version`), RMQTT version

### Bug Report Template

```markdown
## Description
[Clear description of the bug]

## Reproduction Steps
1. Start broker with `...`
2. Connect client using `...`
3. Publish to `...`

## Expected Behavior
[What should happen]

## Actual Behavior
[What actually happens]

## Environment
- OS: [e.g. Ubuntu 24.04]
- Rust: [e.g. 1.89.0]
- RMQTT: [e.g. 0.22.0]
```

### Feature Request Template

```markdown
## Problem
[What problem does this feature solve?]

## Proposed Solution
[How do you envision this working?]

## Alternative Approaches
[Any other ways to solve this?]
```

## Development Setup

### Prerequisites

- Rust 1.89.0+ (install via [rustup](https://rustup.rs/))
- `protoc` (protobuf compiler) — required for tonic/prost builds
  - Linux: `apt install protobuf-compiler`
  - macOS: `brew install protobuf`
  - Windows: download from [protobuf releases](https://github.com/protocolbuffers/protobuf/releases)

### Build

```bash
# Clone the repository
git clone https://github.com/rmqtt/rmqtt.git
cd rmqtt

# Build all crates
cargo build

# Build release binary (all features)
cargo build --release

# Build a specific crate
cargo build -p rmqttd
```

### Code Generation

Some code is auto-generated at build time:

```bash
# Plugin registration code (generated from Cargo.toml metadata)
# Done automatically by rmqtt-bin/build.rs — no manual step needed
```

## Code Style

### Formatting

```bash
# Format all code
cargo fmt --all
```

We enforce `rustfmt` with default settings. All code must be formatted before submission.

### Linting

```bash
# Run clippy on all targets
cargo clippy --all-targets
```

The project maintains **zero clippy warnings** at all times. New code should not introduce warnings.

### Safety

The entire project is `#![deny(unsafe_code)]`. Production code must not contain:

- `unsafe` blocks
- `unwrap()` or `expect()` (test code only)
- `panic!` / `todo!` / `unreachable!` in production paths
- `#[allow(unused_*)]` unless behind a conditional compilation flag

### Commit Messages

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>
```

**Types**: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, `deps`, `ci`

**Scopes** (examples): `core`, `net`, `codec`, `conf`, `test`, `plugin/acl`, `plugin/raft`, `cli`

**Examples**:
```
feat(net): add cert_subject_dn_as_username option for TLS listeners
fix(http-api): add startup synchronization and improve reload handling
deps: upgrade prometheus from 0.13 to 0.14
docs(test): update CLI usage examples for rmqtt-test
```

## Testing

### Running Tests

```bash
# Run all unit tests
cargo test

# Run tests for a specific crate
cargo test -p rmqtt-codec

# Run integration tests using the test harness (requires release build)
cargo build -p rmqttd --release
cargo build -p rmqtt-test --release
./target/release/mqtt_harness --workspace .
```

### Test Suites

The project has two testing layers:

1. **Unit tests** — standard `#[cfg(test)]` modules within each crate
2. **Integration tests** — `rmqtt-test` harness with five suite types:

| Suite | Description |
|-------|-------------|
| `functional_v3` | MQTT 3.1 basic operations |
| `functional_v311` | MQTT 3.1.1 protocol compliance (10 cases) |
| `functional_v5` | MQTT 5.0 protocol compliance (5 cases) |
| `stress` | Connection/publish/fan-out load tests |
| `chaos` | Broker restart, connection storms, slow consumers |

### Interoperability Testing

RMQTT passes the [paho.mqtt.testing](https://github.com/eclipse/paho.mqtt.testing) test suite:

```bash
# MQTT v3.1.1 (11 tests)
python client_test.py

# MQTT v5.0 (24 tests)
python client_test5.py
```

### Performance Benchmarking

```bash
# Build release
cargo build --release

# Run with benchmark configuration
# See docs/en_US/benchmark-testing.md for detailed instructions
```

## Documentation

All documentation follows these standards:

### Code Documentation

- All public API items must have doc comments (`///`)
- Crate-level docs in `lib.rs` explaining purpose and architecture
- Examples in doc comments should be runnable (`rust,no_run` where needed)
- Mark feature-gated items with `#[cfg(feature = "...")]` in docs

### Project Documentation

Located in `docs/` directory with bilingual support:

| Directory | Language |
|-----------|----------|
| `docs/en_US/` | English |
| `docs/zh_CN/` | Simplified Chinese |
| `docs/imgs/` | Shared images |

When adding a new feature, create or update both the English and Chinese documentation.

### README Files

Each crate must have:
- `README.md` — English version with badges, overview, usage, API reference
- `README-CN.md` — Simplified Chinese version, semantically equivalent

## Pull Request Process

1. **Create a feature branch** from the latest `master`:

   ```bash
   git checkout master
   git pull
   git checkout -b feat/my-feature
   ```

2. **Make your changes** following the code style guidelines above.

3. **Verify your changes**:

   ```bash
   # Format
   cargo fmt --all

   # Lint
   cargo clippy --all-targets

   # Test
   cargo test

   # Build (all features)
   cargo build --release
   ```

4. **Commit your changes** with a clear commit message:

   ```bash
   git commit -m "feat(scope): concise description"
   ```

5. **Push and open a PR**:

   ```bash
   git push origin feat/my-feature
   ```

   Then open a pull request on GitHub against the `master` branch.

6. **PR checklist**:
   - [ ] Code compiles without warnings
   - [ ] `cargo clippy --all-targets` passes
   - [ ] `cargo test` passes
   - [ ] New features include tests
   - [ ] Public APIs have doc comments
   - [ ] Documentation updated (English + Chinese)
   - [ ] Commit messages follow conventional format

### PR Review Criteria

Maintainers will review your PR for:

- **Correctness** — does the code do what it claims?
- **Safety** — no unsafe code, no panic paths
- **Performance** — appropriate use of async, locks, and data structures
- **Consistency** — follows existing patterns in the codebase
- **Documentation** — clear doc comments and docs updates

## License

By contributing to RMQTT, you agree that your contributions will be licensed under the [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE) license, at your option.
