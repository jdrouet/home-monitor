# Contributing

## Process

Work follows a scrum-like process, tracked in GitHub Issues. Nothing is
implemented without an issue.

- Every piece of work is a **user story** (or a **spike** for an
  investigation), written in plain language with acceptance criteria. Use the
  issue template.
- Every issue has one `type:`, one `status:`, one `priority:`, and at least one
  `area:` label.
- **Definition of Ready** (`status: ready`): a clear user and benefit, testable
  acceptance criteria, no open questions, small enough for one sprint.
  Otherwise `status: needs refinement`.
- Work starts only on a Ready issue; move it to `status: in progress`.
  `status: blocked` means waiting on something outside the team.
- **Definition of Done**: acceptance criteria met and verified, reviewed,
  merged, and the issue closed by the pull request (`Closes #<n>`).
- Priority: `priority: now` is pulled next, then `next`, then `later`.

## Commit messages

Commit messages must follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```
<type>(<optional scope>): <subject>
```

- Types: `feat`, `fix`, `docs`, `refactor`, `test`, `build`, `ci`, `chore`, `perf`.
- Scope is the service or crate, when one applies: `feat(store): flush write-ahead log daily`.
- Write the subject in the imperative mood, in lower case, with no trailing period.
- Mark breaking changes with `!` after the type or scope (`feat(domain)!: rename metric enum`) and explain them in a `BREAKING CHANGE:` footer.

## Continuous integration

Every pull request runs these checks, and all must pass:

- `cargo fmt --all --check`
- `cargo clippy -- -D warnings` on all targets and features
- Tests, run through `cargo llvm-cov` to produce coverage
- `cargo-machete`, to find unused dependencies
- `cargo hack check --feature-powerset`
- `cargo-crap` 0.9.0, which combines complexity and coverage into a CRAP
  score per function. The threshold is 30 (team decision, issue #3): a
  function scoring above 30 fails the build.

A separate check validates the pull request title and every non-merge commit
against the types listed under Commit messages.

The Rust jobs are skipped while there is no `Cargo.toml` at the repository root.

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
