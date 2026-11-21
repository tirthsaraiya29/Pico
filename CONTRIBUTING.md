# Contributing to Pico

Thank you for your interest in contributing to **Pico** — the high-performance, real-time, locally capable AI voice assistant.

## Code of Conduct

We are committed to providing a welcoming and harassment-free environment for everyone. Please be respectful and constructive in all interactions.

---

## Development Setup

### Prerequisites

1. **Rust Toolchain**: Rust 1.80+ (MSVC on Windows or GCC/Clang on Linux).
2. **C++ Build Tools**: Visual Studio 2022+ C++ Build Tools (Windows) or build-essential (Linux).
3. **Local Inference (Optional for offline testing)**:
   - [Ollama](https://ollama.com) running locally (`ollama run granite4.2` or any supported model).
   - Or [llama.cpp](https://github.com/ggerganov/llama.cpp) binary suite (`llama-server`).

### Building from Source

```bash
# Clone the repository
git clone https://github.com/tirthsaraiya/Pico.git
cd Pico

# Check workspace compilation
cargo check --workspace --all-targets

# Run the test suite
cargo test --workspace --all-features

# Build release binaries
cargo build --workspace --release
```

---

## Coding Standards

1. **Idiomatic Rust**:
   - Write clear, idiomatic Rust code with zero compiler warnings.
   - Run `cargo fmt --all -- --check` prior to committing.
   - Run `cargo clippy --workspace --all-targets --all-features -- -D warnings`.

2. **Error Handling**:
   - Never use `unwrap()` or `expect()` in production runtime paths.
   - Use structured error enums powered by `thiserror`.
   - Propagate errors with rich context.

3. **Asynchronous Runtime**:
   - Never block Tokio worker threads. Use `tokio::task::spawn_blocking` for CPU-intensive tasks or synchronous OS operations.

4. **Security & Validation**:
   - Every tool input must be validated before execution.
   - Respect user permission tiers and sandbox configurations.

---

## Pull Request Guidelines

1. Create a feature branch from `main`:
   ```bash
   git checkout -b feature/my-enhancement
   ```
2. Commit your changes with clear, descriptive commit messages.
3. Ensure all tests pass and formatting is clean.
4. Submit a Pull Request targeting `main`.
