# CLAUDE.md - Substrait TextPlan Project Reference

## Build & Run Commands
- Build: `cargo build`
- Run: `cargo run`
- Release build: `cargo build --release`
- Check compilation: `cargo check`
- Run tests: `cargo test`
- Run single test: `cargo test test_name`

## Lint & Format Commands
- Format code: `cargo fmt`
- Check formatting: `cargo fmt -- --check`
- Run clippy lints: `cargo clippy`
- Run clippy with all features: `cargo clippy --all-features`

## Code Style Guidelines
- **Imports**: Group imports by crate, with std first, then external crates, then internal modules
- **Error handling**: Use `thiserror` for defining error types; prefer `?` operator for propagation
- **Type definitions**: Use descriptive names; for complex shared types, consider type aliases
- **Naming**: Follow Rust conventions (snake_case for functions/variables, CamelCase for types)
- **Documentation**: Document public APIs with doc comments (///)
- **Testing**: Write unit tests in the same file as the code they test, using `#[cfg(test)]` modules
- **File Endings**: All source files must end with a newline character
- **Whitespace**: No trailing whitespace allowed

## Protocol Buffer Handling
- **Serialization**: ALWAYS use `prost-serde` for serialization/deserialization of protobuf, NEVER use `serde_json` directly
- **Protobuf Generation**: Use `prost-build` in the build.rs file to generate Rust code from .proto files
- **Extensions**: Map Google Protobuf types to prost_types equivalents using extern_path in build.rs
- **Empty Type**: Use extern_path for google.protobuf.Empty to ensure proper type resolution
- **Type Paths**: Use the proper full paths when importing types from generated code

## Before Starting New Tasks
- Always check git status before starting new work with `git status`
- Commit or stash pending changes before starting a new task
- Update dependencies regularly with `cargo update`
