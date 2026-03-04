Rust Style Guide Summary
1. Naming

General: Optimize for readability. Be descriptive but concise. Use inclusive language.

Files: .rs. snake_case (my_module.rs). Module names match file names.

Crates: snake_case (my_crate).

Types (Structs/Enums/Traits): PascalCase (MyStruct, MyEnum, MyTrait).

Enum Variants: PascalCase (NotFound, AlreadyExists).

Type Aliases: PascalCase (MyType).

Variables: snake_case (my_var).

Struct Fields: snake_case (user_id).

Constants/Statics: SCREAMING_SNAKE_CASE (MAX_RETRIES).

Generic Parameters: PascalCase single letters or descriptive (T, E, Item).

Lifetimes: Short lowercase with apostrophe ('a, 'de).

Functions: snake_case (get_value()).

Methods: snake_case (as_str(), into_bytes()).

Modules: snake_case (web_server).

Macros: snake_case! (vec!, my_macro!).

2. Modules and Files

General: One module per file when practical. Keep modules cohesive and focused.

Visibility: Items are private by default. Use pub only when necessary.

Re-exports: Use pub use intentionally to define clean public APIs.

Imports (use): Avoid glob imports (use foo::*;) except in tests or controlled preludes.

Import Order:

Standard library (std::)

External crates

Crate-local modules (crate::)
Separate groups with blank lines. Alphabetical within groups.

Path Style: Prefer absolute paths (crate::module::Type) over deep super:: chains.

Inline Modules: Prefer separate files over large inline mod {} blocks.

3. Formatting

Indentation: 4 spaces (rustfmt default).

Line Length: Follow rustfmt defaults. Keep lines readable.

Tooling: Always format with rustfmt. Do not manually fight formatter output.

Braces: if cond { ... }, fn foo() { ... } (K&R style).

Match: One logical pattern per arm. Use braces for multi-line arms.

Trailing Commas: Use in multi-line lists and match arms.

Whitespace: Use to separate logical sections, not every block.

Return: Implicit return preferred for final expression. Use return only for early exit.

Attributes: Place directly above the item (#[derive(...)]).

Macros: Avoid complex formatting; let rustfmt handle layout.

4. Structs and Enums

Structs: Prefer named fields over tuples for clarity.

Tuple Structs: Use for newtype patterns (struct UserId(u64);).

Enums: Use for state machines and domain modeling instead of flags.

Derives: Use #[derive(Debug, Clone, PartialEq, Eq, Hash, Default)] when appropriate.

Field Visibility: Keep fields private unless external access is required.

Construction: Prefer builders or Default for complex initialization.

Drop: Implement Drop only when necessary. Document side effects.

5. Functions

Parameters: Prefer borrowing (&T, &str, &[T]) over owned types when possible.

Ordering: Receiver (&self, &mut self) first, then required inputs.

Outputs: Prefer returning values. Do not use out-parameters.

Errors: Use Result<T, E> for fallible operations. Use Option<T> for absence.

Length: Prefer small functions (~40 lines or fewer).

Panics: Avoid in public APIs except for programmer errors. Document panic conditions.

Inlining: Avoid unnecessary #[inline]. Use only when measured.

Overloading: Not supported. Use trait implementations or clearly named methods.

6. Scoping

Modules: Keep scope minimal. Avoid large root modules.

Imports: Limit use statements to necessary items.

Locals: Declare variables at the narrowest scope possible.

Mutability: Prefer immutable bindings. Use mut only when required.

Statics: Prefer const over static when possible.

Global State: Avoid mutable global state. Use dependency injection.

Thread Local: Use thread_local! sparingly and document reasoning.

7. Modern Rust Features

Edition: Use latest stable Rust edition. Avoid nightly-only features.

Ownership: Prefer move semantics and borrowing over cloning.

Smart Pointers: Use Box<T> (heap allocation), Rc<T> (single-thread shared), Arc<T> (multi-thread shared).

Interior Mutability: Use Cell, RefCell, Mutex, RwLock only when necessary.

Pattern Matching: Prefer match over complex condition chains.

Async/Await: Use async/await for asynchronous code. Do not block in async contexts.

Traits: Prefer trait bounds (where T: Trait) for generic constraints.

Impl Trait: Use impl Trait in return position when appropriate.

Conversions: Prefer From/Into over manual conversion methods.

Casts: Use as cautiously. Prefer explicit conversion traits.

Unsafe: Minimize usage. Document invariants with // SAFETY: comments.

8. Best Practices

Const: Use const where possible for compile-time constants.

Clippy: Run cargo clippy and fix warnings unless explicitly justified.

Testing: Place unit tests in mod tests. Use integration tests in /tests.

Documentation: Use /// for public APIs. Include examples when helpful.

Error Messages: Be descriptive and actionable.

Logging: Use structured logging where possible.

Dependencies: Keep minimal and well-justified.

Comments: Document modules, structs, functions, errors, and safety invariants.

TODOs: Use // TODO(username): description.

BE CONSISTENT. Follow existing project style and rustfmt defaults.
