# Contributing

## Commit messages: Conventional Commits

Every pull request title (and, ideally, every individual commit) follows
[Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>
```

- `scope` is optional.
- Allowed types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`,
  `build`, `ci`, `chore`, `revert`.
- A breaking change is marked either with `!` after the type/scope
  (`feat!: ...`) or with a `BREAKING CHANGE:` footer in the body.

The pull request is merged with squash, so the **pull request title** becomes
the commit on `main` and is what release-please reads to determine the next
version and changelog entry. Get the title right even if the individual
commits on the branch are messy.

## Branch naming

Branches follow `type/short-description`, using the same type vocabulary as
above, for example:

```
fix/release-package-smoke-tests
feat/demo-backend-knx-presence-error
chore/doctor-class-a-bundle
docs/apt-moves-to-deb-snapdog-cc
```

## Local checks before you push

This repository currently uses raw Git hooks via `core.hooksPath`, configured
by:

```
make setup
```

That points Git at `.githooks/` and installs:

- `commit-msg` — rejects a commit message that does not follow the
  Conventional Commits pattern.
- `pre-commit` — checks Rust formatting (`cargo fmt -- --check`) and, when the
  staged files touch WebUI translation files or the macOS String Catalog,
  checks that the affected translations are complete.
- `pre-push` — runs `cargo fmt --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test --workspace`, and `npm run lint` for the WebUI.

You can also run the same checks manually at any time with:

```
make check
```

which runs `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
and the WebUI checks (`npm ci && npm run i18n:check && npm run lint && npm run
typecheck && npm run build`).

If a hook ever blocks you incorrectly, please fix the underlying issue rather
than reaching for `--no-verify` — the same checks run again, and block the
merge, in CI.
