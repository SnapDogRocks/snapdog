# Release process

SnapDog releases are fail-closed and use one compilation per target. Release
tarballs are the source of the binaries subsequently placed, byte-for-byte, in
Debian packages, AUR packages, Homebrew formulae and container images.

## State machine

1. Merging the release-please PR creates the immutable `vX.Y.Z` tag and a **draft**
   GitHub release. Release Please dispatches `release.yml` once at that tag.
2. The workflow refuses non-SemVer refs, non-draft releases, or drafts which
   already contain assets. Per-tag concurrency prevents overlapping runs.
3. WebUI, KNX database and target binaries are built once. Packaging jobs consume
   workflow artifacts; they never compile SnapDog again.
4. Separate jobs install and execute the direct `.deb` files, the signed APT
   repository, both Homebrew formulae and both AUR packages. The `.deb` payloads
   are byte-compared with the original build output. The notarized DMG, archives,
   package files and KNX database are collected and hashed.
5. Only after every verification succeeds are assets uploaded to the draft.
   Uploads never use `--clobber`. A clean download is compared byte-for-byte with
   the verified asset set.
6. The draft is published. Only then may APT, AUR, Homebrew, Sparkle, GHCR and the
   snapdog-os update PR be changed.

There is deliberately no `push: tags` or `release: published` trigger on the
release workflow. Adding either creates a second build and races publication.

## Required secrets

- `RELEASE_PLEASE_APP_CLIENT_ID` and `RELEASE_PLEASE_APP_PRIVATE_KEY`: create the
  release PR, tag, draft and the single workflow dispatch.
- `APT_SIGNING_KEY`: ASCII-armored private key for APT metadata.
- `APT_SIGNING_KEY_FINGERPRINT`: full 40-hex fingerprint. The workflow compares it
  to the imported secret key before signing.
- Existing Apple, Sparkle, AUR, Homebrew, Cloudflare, ETS and snapdog-os secrets.
  The ETS key is mandatory for a release; an unsigned `.knxprod` is rejected.

The current APT signing fingerprint is:

```text
E4AA C210 C8C2 1377 554D  BDE4 0623 E5F3 B437 9FC7
```

The administrative backup is outside the repository at
`~/.config/snapdog-release/`. Never commit the private key. Key rotation requires
publishing the new public key and fingerprint through an authenticated channel
before changing the two secrets.

## Failure and recovery

A failed run leaves an unpublished draft. It never changes package channels. Do
not overwrite or retag an existing release. Diagnose the failed gate, then delete
only the incomplete draft assets through the GitHub UI before manually dispatching
`release.yml` at the existing tag. The workflow refuses a retry while any asset is
present, so recovery cannot silently mix outputs from different runs.

Once a draft has been published, it and its tag are immutable operational records.
Fixes require a new version and a new release-please PR.

## Local validation

Before merging release-process changes, run:

```bash
go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.7 -ignore SC2015
git diff --check
```

The real installation gates run only in the release workflow because they require
all target artifacts, Apple notarization and the repository signing secrets.
