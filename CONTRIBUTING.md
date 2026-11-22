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

All PRs must pass `make ci` (formatting, `check`, `lint` on every feature combination, all
tests, and the documentation build) and branch naming validation. See `AGENTS.md` for the code
rules.

### Releasing

Releases are generated. Give every pull request a `type(scope): description` title and merge with squash; release-please keeps a release pull request open with the next version and `CHANGELOG.md`. Merging it tags `vX.Y.Z`, which starts the release workflow. See `AGENTS.md`.
