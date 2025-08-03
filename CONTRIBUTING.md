# Contributing to autopulsed

Thank you for your interest in contributing to autopulsed! This document
provides guidelines for contributing to the project.

## Code of Conduct

Please be respectful and constructive in all interactions.

## How to Contribute

### Reporting Issues

- Check if the issue already exists
- Provide clear description and steps to reproduce
- Include relevant system information

### Submitting Pull Requests

1. Fork the repository
2. Create a feature branch from `master`
3. Make your changes
4. Run tests and linters
5. Submit a pull request with clear description

## Development Setup

### Prerequisites

- Rust toolchain (stable)
- libpulse-dev package
- PulseAudio server

### Building

```bash
cargo build
cargo build --release
```

### Testing

```bash
cargo test
cargo clippy
cargo fmt --check
```

## Code Style

- Follow Rust standard style guidelines
- Use rustfmt for formatting
- Address Clippy warnings
- See rustfmt.toml and clippy.toml for project settings

## Commit Messages

Follow this format:

```
Short summary of changes (max 50 chars)

Detailed explanation of what and why (if needed).
Each line should be 76 characters or less.

Fixes #123
```

Examples:
- `Add device priority configuration support`
- `Fix race condition in device enumeration`
- `Update dependencies to latest versions`

## License and Copyright

### Copyright Retention

**Contributors retain copyright to their contributions.** By submitting
a pull request, you agree to license your contribution under the GNU
Affero General Public License version 3 (AGPL-3.0) or later.

### Adding Copyright Headers

When you make significant contributions to a file, add your name to the
copyright header:

```rust
// autopulsed - A daemon for configuring PulseAudio automatically
// Copyright (C) 2025  Flokart World, Inc.
// Copyright (C) 2025  Your Name <your.email@example.com>
```

### Developer Certificate of Origin

By contributing to this project, you certify that:

1. The contribution is your original work, or
2. You have the right to submit it under AGPL-3.0

This is affirmed by adding a `Signed-off-by` line to your commit:

```
Signed-off-by: Your Name <your.email@example.com>
```

You can add this automatically using `git commit -s`.

## Questions?

Feel free to open an issue for any questions about contributing.
