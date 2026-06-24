---
id: rust10x
name: rust10x
description: Guidelines and best practices for writing high-performance, robust, and beautifully structured Rust applications following the Rust10x standard.
---

# Rust10x (Jeremy Chone) Standard

## Design, Layout & Comments

*   **Forbid Unsafe**: Workspace lints must enforce memory safety: `[lints.rust] unsafe_code = "forbid"`.
*   **Layout Order (Top-to-Bottom)**:
    1.  `// region:    --- Types` (Public container types to leaf types. Double-space commented fields).
    2.  Public Implementations & Module Functions.
    3.  `// region:    --- Support` (Private types and helpers).
    4.  `// region:    --- Tests` (Unit tests at the bottom).
*   **Imports**: Import macros explicitly (`use lopdf::dictionary;`) instead of qualified calls.
*   **Iterators**: For type `T`, implement `IntoIterator` for `T` and `&T` under `// region:    --- Iterator Implementations`, with a companion `pub fn iter(&self)` under `impl T`.

## Error Handling (No `thiserror` / `anyhow`)

Flatten `pub use error::{Error, Result};` via a custom `mod error;` using `derive_more`:

```rust
use derive_more::{Display, From};
pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Display, From)]
#[display("{self:?}")]
pub enum Error {
    #[from(String, &String, &str)]
    Custom(String),
    #[from]
    Io(std::io::Error),
}
impl Error {
    pub fn custom(val: impl Into<String>) -> Self { Self::Custom(val.into()) }
}
impl std::error::Error for Error {}
```

*   **In Tests/Examples**: Use `type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;`.

## Testing & CLI Best Practices

*   **Testing**: No `unwrap()` or `expect()`. Use `.ok_or()?` or `Result` propagation.
*   **Test Naming**: Name tests as `test_[module_path_name]_[function_name]_[variant]()`.
*   **Phases**: Structure test assertions inside commented blocks: `// -- Setup & Fixtures`, `// -- Exec`, and `// -- Check`.
*   **CLI Separation**: CLI commands in `src/cli/` must parse/format only. Keep domain logic inside `src/handlers/` completely free of CLI/clap dependencies.
