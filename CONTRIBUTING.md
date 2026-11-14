# Contributing

## Commit messages

Commit messages must follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```
<type>(<optional scope>): <subject>
```

- Types: `feat`, `fix`, `docs`, `refactor`, `test`, `build`, `ci`, `chore`, `perf`.
- Scope is the service or crate, when one applies: `feat(store): flush write-ahead log daily`.
- Write the subject in the imperative mood, in lower case, with no trailing period.
- Mark breaking changes with `!` after the type or scope (`feat(domain)!: rename metric enum`) and explain them in a `BREAKING CHANGE:` footer.

## Code style

Rust code follows the [Google Rust Style Guide](https://google.github.io/styleguide/rustguide.html),
which builds on the official Rust Style Guide and API Guidelines. `cargo fmt`
and `cargo clippy -- -D warnings` must pass.

Rules that matter most here, mainly for build time:

- Prefer small crates. Split a crate when it grows past one concern, so Cargo
  can build crates in parallel and cache them.
- Name files and leaf modules after their primary item. Non-leaf modules hold
  only `mod` declarations and re-exports.
- No glob imports. Each `use` statement imports from one module.
- Regular Rust first, then `macro_rules!`, then proc macros as a last resort.
  Derives from established crates (`serde`, `thiserror`) are allowed.
- Prefer `expect("reason")` over `unwrap()`. Prefer pattern matching over
  check-then-unwrap.
- Prefer enums over `bool` parameters, named fields over tuples, newtypes over
  type aliases for domain values.
- Stable Rust only. No unstable features.

Project exceptions:

- Rules tagged *[chrome]* or *[Fuchsia]* in the guide do not apply. In
  particular, services use `async`/`.await` with `tokio`.
- `Arc` is allowed for state shared across async tasks, as `axum` and `tokio`
  require. Avoid it elsewhere.
- `dyn Trait` is forbidden unless generics cannot express the code, for
  example a collection of different implementations chosen at runtime, or a
  library API that requires a trait object. Each use needs a comment saying
  why. Use cases take their ports as generic parameters.
