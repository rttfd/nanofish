# Contributing to nanofish

## Git Workflow

### Protected Main

- **`main` is protected** — never push directly to `main`.
- All changes must go through a pull request.
- PRs require CI to pass before merge.

### Branch Naming

All branches must follow the pattern:

```
<type>/<short-description>
```

**Allowed types:**
- `feat` — new features
- `fix` — bug fixes
- `chore` — maintenance, dependencies, tooling
- `docs` — documentation only
- `refactor` — code restructuring without functional change
- `test` — adding or updating tests
- `release` — version bumps and release prep

**Examples:**
- `feat/response-constructors`
- `fix/header-parsing`
- `chore/bump-embassy-net`

### Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

<body>
```

**Types:** `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `ci`

### CI Requirements

All PRs must pass:
- `cargo fmt` check
- `cargo clippy -- -D warnings`
- `cargo test`
- Branch naming validation

### Releasing

1. Update `CHANGELOG.md` under `[Unreleased]`
2. Bump version in `Cargo.toml`
3. Create a `release/vX.Y.Z` branch
4. Open PR, merge to `main`
5. Tag `main` with `vX.Y.Z`
6. Push tag (triggers release workflow)
