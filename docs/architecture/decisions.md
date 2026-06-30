# Architecture Decision Records

This file is the formal log of architecture decisions for SnapDog. Earlier
decisions (ADR-001…018) are referenced inline across the code and RFCs and will be
back-filled here over time; the formal log begins at ADR-019.

Each entry follows a MADR-minimal shape: **Status · Context · Decision ·
Consequences · References**.

---

## ADR-019 — Pin snapcast at 0.16.1 until the seam firewall is complete

- **Status:** Accepted (2026-06-30)
- **Deciders:** maintainer
- **Tracking:** RFC 0003 `IT-T08` (this decision), `IT-NG-05` (the upgrade itself)

### Context

`snapcast-rs` 0.17 carries breaking changes to the `snapcast-server` /
`snapcast-proto` API (e.g. the `init()` / `state()` removal that already bit once
during refactoring). SnapDog currently pins **0.16.1** from crates.io
(`snapcast-server`, `snapcast-client`, `snapcast-proto` in the workspace
`Cargo.toml`), and a local `snapcast-rs` 0.17 checkout exists as path-deps but is
**not** adopted.

The risk that makes this decision necessary: the embedded-server JSON-RPC control
seam shares **no types** between snapdog and the dependency — drift is therefore
**compiler-invisible** and surfaces only at runtime. A breaking version jump made
without a regression net would land silently.

### Decision

1. **Stay pinned at snapcast 0.16.1** for `snapcast-server` / `snapcast-client` /
   `snapcast-proto`; do **not** adopt the local 0.17 path-deps yet.
2. **Keep the snapdog seam firewall version-agnostic** — it stands on its own
   golden JSON-RPC request vectors, the `ServerEvent → SnapcastEvent` map coverage,
   and the mocked `SnapcastBackend` trait, none of which depend on a snapcast 0.17
   test harness.
3. **Sequencing rule:** the 0.16.1 → 0.17 upgrade (`IT-NG-05`) happens **only after**
   the seam firewall is complete and green — `IT-T52` (event map), `IT-T54` (17
   JSON-RPC vectors), `IT-T55` (F32 sender), **and `IT-T73`** (feature build-smoke
   matrix). The upgrade is then performed as separate work, behind that firewall,
   so any wire/signature drift fails a test rather than shipping.

### Consequences

- The breaking jump is deferred but de-risked: the regression net catches
  method/params drift (golden vectors), event-mapping drift (exhaustive coverage),
  and gross signature breaks across every feature combination (build-smoke).
- SnapDog tracks a stable crates.io release rather than a moving local checkout —
  reproducible builds, no surprise breakage from upstream development.
- Cost: SnapDog forgoes 0.17 features/fixes until the upgrade is scheduled. Accepted
  — correctness of the audio control plane outranks early adoption.

### References

- RFC `docs/rfcs/0003-integration-test-suite.md` — §9.1 crate-contract firewall,
  §15 roadmap `IT-NG-05`, tasks `IT-T52` / `IT-T54` / `IT-T55` / `IT-T73`.
- Workspace `Cargo.toml` — the `snapcast-*` 0.16.1 pins.
- `snapdog/tests/snapcast_rpc.rs` — the golden JSON-RPC wire firewall.
