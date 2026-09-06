# Dependabot review — 2026-09-06

Scope: open Dependabot PRs in SnapDog and SnapDog-OS, followed by stable crate
updates in the SnapDog Rust workspace. PR diffs, dependency/release information
and GitHub check results were inspected. This is not a full audit of every
upstream package's source. No PR was approved, closed or merged by this work.

## SnapDog

| PR | Change | Assessment / next step |
| --- | --- | --- |
| [#221](https://github.com/SnapDogRocks/snapdog/pull/221) | rust-cache Action SHA | Relevant checks pass and GitHub reports CLEAN. Target resolves to the signed upstream 2.9.2 commit. Only Action references change. Merge separately from application dependencies. |
| [#222](https://github.com/SnapDogRocks/snapdog/pull/222) | release-please Action SHA | Relevant checks pass and GitHub reports CLEAN. Target resolves to the signed upstream 5.0.0 commit; its Node 24 runtime is suitable for the hosted ubuntu-latest job and the configured inputs still exist. Merge separately and verify the next release-PR run. |
| [#229](https://github.com/SnapDogRocks/snapdog/pull/229) | Next.js 16.3.4, shadcn 4.19.1 | WebUI checks pass on its head, but the branch is BEHIND. Refresh against main and coordinate with #230 so Next.js and its ESLint configuration stay aligned. |
| [#230](https://github.com/SnapDogRocks/snapdog/pull/230) | eslint-config-next 16.3.4 | WebUI checks pass on its head, but the branch is BEHIND. Refresh and validate together with #229. |
| [#232](https://github.com/SnapDogRocks/snapdog/pull/232) | futures-util, mdns-sd, toml, tower-http | Relevant checks pass, but the branch is BEHIND. The broader stable-crate update supersedes all four bumps; avoid merging its older lockfile over the refreshed one. Close as superseded only after the replacement lands. |

The public upstream commit endpoints resolved both new Action SHAs, but the
old-to-new GitHub compare endpoints returned 404. A complete upstream diff was
therefore not independently audited. Passing workflow lint is not a test of the
release action's behavior during a real release.

## SnapDog-OS

| PR | Change | Assessment / next step |
| --- | --- | --- |
| [#167](https://github.com/SnapDogRocks/snapdog-os/pull/167) | npm dependency group | Checks pass on its head, but the branch is BEHIND. Refresh after the targeted lockfile fixes; rerun checks rather than treating old successful runs as proof for a merged dependency graph. |
| [#169](https://github.com/SnapDogRocks/snapdog-os/pull/169) | fast-uri 3.1.7 | Initially waiting for ARM cross-compilation; subsequently merged by the repository's existing github-actions auto-merge on 2026-09-05 at 23:32:25 UTC. |
| [#170](https://github.com/SnapDogRocks/snapdog-os/pull/170) | postcss-selector-parser 7.1.6 | Targeted lockfile-only parser security update. Earlier checks passed; after #169 merged, GitHub reported BLOCKED while the refreshed checks were pending. Prioritize once current-head checks pass. |

Suggested order: targeted SnapDog-OS parser update, refreshed OS npm group;
SnapDog Action updates separately, aligned WebUI updates, then the broader Rust
update after its own checks. These states are a review-time snapshot, not a
standing authorization for automatic merges.

For Rust compatibility holds and the three remaining security advisories, see
[dependency maintenance](dependencies.md).
