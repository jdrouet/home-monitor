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
