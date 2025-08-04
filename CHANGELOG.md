# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] - 2025-08-04

### Added
- `--server` option to specify custom PulseAudio server address (e.g., `--server unix:/run/user/1000/pulse/native`)
- GitHub Actions CI workflow for automated testing and code quality checks
- Pre-commit hooks to ensure code quality before commits

### Developer Experience
- New CI pipeline with:
  - Format checking (rustfmt)
  - Linting (clippy)
  - Unit tests
  - Integration tests with isolated PulseAudio
  - Testing on Rust 1.85.0 (minimum version) and stable
  - Release binary builds
- Pre-commit hooks validate staged content to prevent formatting issues

## [0.1.0] - 2025-08-03

### Added
- Initial release of autopulsed - A daemon for configuring PulseAudio automatically
- Real-time monitoring of PulseAudio device changes via event subscription
- Automatic default sink/source switching based on device priority
- YAML configuration file support for device detection rules
- Device matching using PulseAudio property lists
- Support for multiple device configurations with priority settings
- Comprehensive logging with configurable verbosity levels
- Unit tests for core business logic (device matching and priority selection)
- Integration tests with isolated PulseAudio instance
- AGPL-3.0 license with proper copyright headers

### Project Scope
- PulseAudio-specific daemon (not for ALSA/JACK/PipeWire)
- Linux platform (tested on Debian/Ubuntu)
- CLI tool with YAML configuration (no GUI)

### Features
- **Device Detection**: Automatically detects audio devices based on their properties
- **Priority Management**: Sets default devices based on configured priority (lower number = higher priority)
- **Hot-plug Support**: Responds immediately to device connection/disconnection events
- **Flexible Configuration**: Supports complex device matching rules via YAML
- **Dual Device Types**: Manages both audio sinks (outputs) and sources (inputs)

### Technical Details
- Written in Rust for performance and safety
- Uses libpulse-binding for PulseAudio integration
- Single-threaded event-driven architecture with `Rc<RefCell<>>` state management
- Careful handling of circular references with weak pointers
- Comprehensive error handling and logging

### Testing
- 7 unit tests covering device matching and priority logic
- 1 integration test with isolated PulseAudio server
- Log-based verification for daemon behavior
- No interference with system audio during tests

### Development Setup
- Rust stable toolchain support
- Clippy and rustfmt configuration for code quality
- CONTRIBUTING.md with copyright retention policy
- Enforced coding conventions via linter configuration
