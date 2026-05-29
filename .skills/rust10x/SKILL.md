---
name: rust10x
description: Guidelines and best practices for writing high-performance, robust, and beautifully structured Rust applications following the Rust10x (Jeremy Chone) standard. Use this skill whenever the user asks to write, edit, refactor, or structure Rust code, design error-handling systems, write unit tests, structure Cargo.toml, or organize files in Rust projects.
---

# Rust10x (Jeremy Chone) Standard Skill

This skill contains the comprehensive set of professional design, coding, commenting, testing, and error-handling standards under the Rust10x paradigm.

## General Design & Formatting Rules

- **Default Project Target**: When starting a new project without specifying "xp" or "library", assume it is a binary project.
- **Comment Spacing**: If a struct field or enum variant has a comment or attribute preceding it, add an empty line before it for readability (unless it is the first field/variant).
- **Macro Imports**: Always import macros explicitly with `use` rather than qualified.
  - *Do not write*: `let dict = lopdf::dictionary! { ... };`
  - *Instead write*: `use lopdf::dictionary; let dict = dictionary! { ... };`
- **Iterator Implementations**: When implementing iterators for a type `T`, always implement:
  - `impl IntoIterator for T`
  - `impl IntoIterator for &T`
  - Place them inside a `// region:    --- Iterator Implementations` block, and add a companion `pub fn iter(&self)` implementation inside the `impl T` block.

## Single-File Code Layout

All Rust modules and source files must organize elements from top to bottom as follows:
1. **Public Types**: Grouped under `// region:    --- Types` block (from main container types to leaf types).
2. **Public Implementations & Functions**: Module implementations.
3. **Private Implementations & Types**: Grouped under `// region:    --- Support` block.
4. **Unit Tests**: Grouped under `// region:    --- Tests` block at the very bottom.

## Comments & Code Regions

Do not add arbitrary comment styles for delimiting. Follow these two standard tiers strictly:

1. **Code Regions**: For grouping larger logical blocks of code (such as type definitions, helper modules, or test sets).
   ```rust
   // region:    --- Region Name

   // endregion: --- Region Name
   ```
   *Note: Exactly four spaces after the first slash-slash (`// region:    `), followed by three dashes and a space (`--- `).*

2. **Code Section Markers**: For inline visual headers without end markers.
   ```rust
   // -- Section Name
   ```
   *Note: Exactly two dashes.*

## Error Handling Pattern (No `thiserror` / `anyhow`)

To minimize dependencies and optimize compilation performance, Rust10x uses custom, structured Rust errors and standard boxed errors for testing.

### 1. In Production, Application & Library Code
* Define a `mod error;` inside `lib.rs` or `main.rs` and flatten via `pub use error::{Error, Result};`.
* The `error.rs` file must follow this exact template:
  ```rust
  use derive_more::{Display, From};

  pub type Result<T> = core::result::Result<T, Error>;

  #[derive(Debug, Display, From)]
  #[display("{self:?}")]
  pub enum Error {
      #[from(String, &String, &str)]
      Custom(String),

      // -- Externals
      #[from]
      Io(std::io::Error),
  }

  // region:    --- Custom

  impl Error {
      pub fn custom_from_err(err: impl std::error::Error) -> Self {
          Self::Custom(err.to_string())
      }

      pub fn custom(val: impl Into<String>) -> Self {
          Self::Custom(val.into())
      }
  }

  // endregion: --- Custom

  // region:    --- Error Boilerplate

  impl std::error::Error for Error {}

  // endregion: --- Error Boilerplate
  ```

### 2. In Tests & Examples
* Use the following boxed-error dynamic type alias:
  ```rust
  type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;
  ```

## Cargo.toml Dependency Management

* **Zero Unsafe**: Always enforce strict memory safety via workspace lints:
  ```toml
  [lints.rust]
  unsafe_code = "forbid"
  ```
* **Dependency Grouping**: Segment dependencies clearly using category headers without empty lines above/below them:
  ```toml
  [dependencies]
  # -- Async
  tokio = { version = "1", features = ["full"] }
  # -- Json
  serde = { version = "1", features = ["derive"] }
  # -- Others
  derive_more = { version = "2", features = ["from", "display"] }
  ```

## CLI / Subcommand Separation of Concerns

* Keep the CLI layer thin and focused only on parsing, verification, and formatting within `src/cli/` (e.g. `cmd.rs`, `executor.rs`, `exec_<subcommand>.rs`).
* Core business/domain logic must reside completely within `src/handlers/` and be fully free of CLI dependencies (no `clap` references).

## Testing Best Practices

* **No Unwraps**: Never use `unwrap()` or `expect()`. Use `.ok_or()?` or `Result` propagation.
* **Test Functions**: Must match the format `test_[module_path_name]_[function_name]_[variant]()`.
* **Standard Phases**: Every test function must be strictly split into commented phases:
  ```rust
  #[test]
  fn test_support_text_replace_markers_simple() -> Result<()> {
      // -- Setup & Fixtures
      let input = "hello";

      // -- Exec
      let output = replace_markers(input);

      // -- Check
      assert_eq!(output, "hello");
      Ok(())
  }
  ```
