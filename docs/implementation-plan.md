# mcp-vault-wrap Implementation Plan

_March 2026 — Derived from MVP Architecture Contract (Revision 8), Product Spec, and Technical Design Spec_

---

## 1) Purpose

This document defines milestones, task sequencing, dependencies, and delivery checkpoints for the MVP implementation. Every task traces to the technical design spec (D-\*), which in turn traces to the architecture contract (C-\*) and product spec (P-\*).

### Resolved Sequencing Decisions

These were resolved before drafting and constrain the plan:

| # | Decision | Rationale |
|---|----------|-----------|
| S-1 | **Strict runtime-first gate** — full `run` spec including sampling carve-out and permission checks must pass before migration work begins | Carve-out is architecturally load-bearing (only place the relay parses message content); validating it early avoids rework |
| S-2 | **Config read+write together** — TOML model built in one pass | Serde model is bidirectional; splitting creates drift risk from hand-crafted fixtures |
| S-3 | **Gate with InMemoryBackend** — runtime gate evidence uses InMemory, not Keychain | Proves architecture in CI; Keychain is a trait impl validated separately |
| S-4 | **`add`/`remove` before runtime gate** — useful for manual Keychain smoke testing | Tiny scope (~20 lines of logic each), provides manual seed path for `run` on macOS |
| S-5 | **CLI stubs in one pass** — all five subcommands wired upfront | Catches clap conflicts early; near-zero cost for a five-command CLI |
| S-6 | **In-process integration tests** — runtime gate integration test exercises `run` orchestration function with injected `InMemoryBackend` and real child process, not the compiled binary | Trait boundaries are the test seams; binary-level testing with real Keychain is covered by M7 smoke tests |

---

## 2) Milestone Overview

```
M1  Project Scaffold          ─┐
M2  Foundation Modules         ├── shared infrastructure
M3  add / remove Commands     ─┘
M4  Runtime Path (run)         ── RUNTIME-FIRST GATE (§10)
M5  Migration (migrate)
M6  Doctor
M7  Integration Hardening      ── Keychain smoke tests, CI, polish
```

Each milestone ends with a delivery checkpoint that defines what must be true before proceeding.

---

## 3) Milestones

### M1 — Project Scaffold

**Goal:** Compilable binary with CLI dispatch, CI pipeline, and project infrastructure.

| ID | Task | Traces To | Depends On |
|----|------|-----------|------------|
| I-01 | `cargo init`, workspace layout matching tech spec §2 module map | C-§8, Tech spec §2 | — |
| I-02 | `clap` derive setup with all five subcommands (`run`, `add`, `remove`, `migrate`, `doctor`) as `todo!()` stubs; `--version`; `--config` flag on `run`, `migrate`, and `doctor` subcommands only | C-§5, P-§2 | I-01 |
| I-03 | CI pipeline: build, test, `cargo fmt --check`, `cargo clippy`, `cargo audit` | C-§8, Design Decisions §9 | I-01 |
| I-04 | License files (MIT + Apache-2.0), `.gitignore`, `rust-toolchain.toml` | C-§8, Design Decisions §9 | I-01 |

**Checkpoint:** `cargo build` succeeds. All five subcommands parse and hit `todo!()`. CI passes green.

---

### M2 — Foundation Modules

**Goal:** All shared types, traits, and implementations that both `run` and `migrate` depend on.

| ID | Task | Traces To | Depends On |
|----|------|-----------|------------|
| I-05 | `validate.rs` — `is_valid_name()` with unit tests covering accept/reject boundaries | C-§4 | I-01 |
| I-06 | `secret/mod.rs` — `SecretBackend` trait with `authenticate`, `get`, `set`, `delete`, `exists` | D-05, C-§4.1 | I-01 |
| I-07 | `secret/memory.rs` — `InMemoryBackend` with full test coverage | D-05, C-§4.1 | I-06 |
| I-08 | `secret/keychain.rs` — `KeychainBackend` using `security-framework`; maps `(profile, name)` to service name `mcp-vault-wrap.<profile>.<name>` | D-05, C-§4.1, C-§4.3 | I-06 |
| I-09 | `config/mod.rs` — `RelayConfig`, `ProfileConfig`, `ServerConfig` structs with serde derives; TOML serialization and deserialization; round-trip tests | D-06, C-§4.4 | I-01 |
| I-10 | `config/validate.rs` — config version check (`== 1`), file permission check (`600`), vault URI validation in `secret_env`, rejection of `vault://` in `env` | D-06, C-§4.4, P-§4 | I-09 |
| I-11 | `VaultUri` struct and parser — `vault://<profile>/<secret-name>` with name validation on both components | D-06, C-§4.3 | I-05 |
| I-12 | `registry.rs` — `RegistryEntry`, `EnvClassification`, static registry with `github` and `slack` entries; lookup-by-name function | D-08, C-§4, C-§7 | I-01 |
| I-13 | `inject/mod.rs` + `inject/env.rs` — `Injector` trait and `EnvInjector` implementation with tests | D-09, Design Decisions §4 | I-01 |

**Checkpoint:** All foundation types compile, serialize/deserialize correctly, and have passing unit tests. `InMemoryBackend` passes full trait contract tests. `KeychainBackend` compiles (manual macOS verification deferred to M7).

---

### M3 — add / remove Commands

**Goal:** Working `add` and `remove` commands. Provides manual Keychain seed path for `run` smoke testing.

| ID | Task | Traces To | Depends On |
|----|------|-----------|------------|
| I-14 | `add` command: validate names, read secret from stdin (interactive + pipe), `--force` flag, call `backend.set()`, print confirmation | D-05, P-§5, C-§5 `add` | I-02, I-05, I-06 |
| I-15 | `remove` command: validate names, check `backend.exists()`, call `backend.delete()`, print confirmation | D-05, P-§6, C-§5 `remove` | I-02, I-05, I-06 |
| I-16 | Error messages matching product spec prescribed strings for both commands | P-§5, P-§6 | I-14, I-15 |
| I-17 | Tests for `add`/`remove` using `InMemoryBackend`: success paths, duplicate detection, `--force` overwrite, invalid names | D-05, P-§5, P-§6 | I-07, I-14, I-15 |

**Checkpoint:** `add` and `remove` work against `InMemoryBackend` in tests. Error messages match product spec. On macOS, manual `add`/`remove` against real Keychain works (informal verification, not gated).

---

### M4 — Runtime Path (`run`) — RUNTIME-FIRST GATE

**Goal:** Full `run` command per product spec §4, validated end-to-end. This is the gate required by architecture contract §10.

| ID | Task | Traces To | Depends On |
|----|------|-----------|------------|
| I-18 | `transport/mod.rs` + `transport/stdio.rs` — async `Transport` trait, `StdioTransport` wrapping `BufReader` for newline-delimited JSON framing | D-02, C-§4.2 | I-01 |
| I-19 | `relay/mod.rs` — proxy loop using `tokio::select!` over two transports; message forwarding in both directions; boolean flag for first-message tracking | D-01, D-02, C-§4.2 | I-18 |
| I-20 | `relay/carveouts.rs` — sampling carve-out: parse `initialize` request, strip `params.capabilities.sampling`, re-serialize, forward | D-03, C-§4.2, C-§5 `run` | I-19 |
| I-21 | `run` orchestration: load config → validate version + permissions → look up server in `profile.default` (MVP only; fail if `profile.default` is missing) → parse vault URIs → authenticate → resolve secrets → build Command → inject → set env → spawn → wire transports (including `Arc` host-side handle seam, D-10) → enter proxy loop | D-01, D-10, P-§4, Tech spec §6.1 | I-09, I-10, I-11, I-06, I-13, I-18, I-19, I-20 |
| I-22 | Downstream process lifecycle: `Stdio::piped()` for stdin/stdout, `Stdio::inherit()` for stderr passthrough; exit with child's exit code | Tech spec §9, P-§4 | I-21 |
| I-23 | Signal forwarding: SIGTERM/SIGINT → forward to child, wait for exit | Tech spec §9, P-§4 | I-22 |
| I-24 | `--verbose` output: startup messages to stderr, prefixed `[mcp-vault-wrap]`; visible warning that verbose mode may log sensitive metadata | P-§4, Design Decisions §8 | I-21 |
| I-25 | Error messages matching product spec §4 failure cases: missing server, missing config, bad permissions, bad version, missing secret, Keychain inaccessible, spawn failure, downstream exit | P-§4 | I-21 |
| I-26 | Relay proxy tests using `InMemoryBackend` + mock transport: passthrough correctness, sampling carve-out fires on first message only, message ordering preserved, unknown JSON-RPC messages forwarded as-is (D-11); verify framing/transport errors trigger relay shutdown rather than passthrough; verify no secret values appear in relay stderr output under any verbosity level (C-§6) | D-01, D-03, D-11, C-§4.2, C-§6 | I-07, I-18, I-19, I-20 |
| I-27 | In-process integration test: exercise `run` orchestration function with `InMemoryBackend`, real `StdioTransport`, and a trivial echo MCP server child process — verify resolve → spawn → relay round-trip, stderr passthrough, and exit-code mirroring | C-§10 gate evidence | I-21, I-26 |

**Checkpoint (RUNTIME-FIRST GATE):**
- Relay proxy tests pass: passthrough, sampling carve-out, message ordering, unknown message forwarding, framing error shutdown, secret-never-logged (C-§6)
- Integration test proves resolve → spawn → relay end-to-end, including stderr passthrough and exit-code mirroring
- `run` error paths produce product-spec-matching messages
- Permission check rejects `644`, accepts `600`
- Config version check rejects version `!= 1`
- Signal forwarding implemented (I-23); manual verification acceptable for signal tests that are impractical in CI
- All other evidence is CI-provable (InMemoryBackend, no Keychain dependency)

**Gate evidence required before proceeding to M5.**

---

### M5 — Migration (`migrate`)

**Goal:** Full `migrate` command per product spec §3, all four phases.

| ID | Task | Traces To | Depends On |
|----|------|-----------|------------|
| I-28 | `host/mod.rs` — `HostConfig` trait with `parse()` and `write()`; `HostConfigData` and `HostServerEntry` structs | D-07, C-§3, C-§5 `migrate` | I-01 |
| I-29 | `host/claude_desktop.rs` — Claude Desktop JSON parser: read `mcpServers`, extract `command`/`args`/`env`, preserve `raw` for round-trip; write back modifying only migrated server entries | D-07, P-§3 | I-28 |
| I-30 | `migrate.rs` Phase 1 (Validate): `--host` flag resolution (accept `claude-desktop`, reject others with actionable error), host config parse, server+registry dual-name lookup, env var name validation, env var completeness check against registry, relay TOML existence check, Keychain accessibility, backup creation (with .bak collision check). Migration writes to `profile.default` only (MVP constraint, C-§3). | D-04, C-§3, C-§4, C-§5 `migrate`, P-§3 | I-28, I-29, I-12, I-05, I-06, I-09 |
| I-31 | `migrate.rs` Phase 2 (Write secrets): iterate classified Secret env vars, call `backend.set()` for each | D-04, C-§5 `migrate` | I-30 |
| I-32 | `migrate.rs` Phase 3 (Write relay TOML): build `RelayConfig` from registry classification, write with permissions `600`, create config directory `700` if needed | D-06, C-§4.4, C-§5 `migrate`, Tech spec §10 | I-30, I-09 |
| I-33 | `migrate.rs` Phase 4 (Rewrite host config): replace migrated server entries with `mcp-vault-wrap run <name>`, remove env, write via `HostConfig::write()` | D-07, C-§5 `migrate`, P-§3 | I-30, I-28 |
| I-34 | `--dry-run` mode: full Phase 1 validation, Phases 2-4 print prefixed output without writing | D-04, C-§5 `migrate`, P-§3 | I-30, I-31, I-32, I-33 |
| I-35 | Error messages matching product spec §3 failure cases: server not found, no registry entry, unrecognized env vars, TOML exists, invalid name, Keychain failure, unsupported host | P-§3 | I-30 |
| I-36 | Migration tests using `InMemoryBackend` + in-memory host config: success path (github + slack), all failure cases from P-§3, dry-run output verification, backup creation, idempotent rerun after Phase 2 failure, .bak collision refusal | D-04, C-§5 `migrate`, P-§3 | I-07, I-30–I-35 |

**Checkpoint:**
- Migration succeeds for `github` and `slack` server entries
- Generated relay TOML round-trips correctly and is loadable by `run`
- Host config rewrite produces valid JSON with only migrated entries changed
- All product-spec failure messages verified
- Dry-run output matches product spec format
- Backup and recovery paths tested

---

### M6 — Doctor

**Goal:** Diagnostic command per product spec §7.

| ID | Task | Traces To | Depends On |
|----|------|-----------|------------|
| I-37 | `doctor` command: config existence + parse + version + permissions check, Keychain accessibility, per-server secret presence (via `exists()`, never `get()`), non-secret env reporting. Iterates `profile.default` servers only (MVP constraint, C-§3). | D-05, D-06, P-§7, C-§5 `doctor`, Tech spec §6.5, Tech spec §10 | I-02, I-09, I-10, I-06 |
| I-38 | Output formatting: checkmark/cross per check, actionable fix instructions, summary line, exit code | P-§7 | I-37 |
| I-39 | Tests: all-pass output, missing secret, missing config, bad permissions, inaccessible Keychain | D-05, P-§7 | I-07, I-37 |

**Checkpoint:**
- Doctor output matches all product spec §7 examples
- Exit code 0 on all-pass, non-zero on any issue
- Uses `exists()` only — never loads secret values

---

### M7 — Integration Hardening

**Goal:** Prove the real platform path, CI hardening, release preparation.

| ID | Task | Traces To | Depends On |
|----|------|-----------|------------|
| I-40 | Manual macOS smoke test: `add` → `run` → relay traffic with a real MCP server (GitHub or Slack), real Keychain | C-§10, C-§11 Behavior Gates | I-08, I-21, I-14 |
| I-41 | Manual macOS smoke test: `migrate` end-to-end on a real Claude Desktop config, verify host launches `mcp-vault-wrap run` correctly | C-§11 Behavior Gates | I-40 |
| I-42 | `cargo audit` clean, dependency pins verified, `Cargo.lock` committed | C-§8, Design Decisions §9 | I-01 |
| I-43 | Security model section in repo docs matching C-§6 claims verbatim | C-§6 | — |
| I-44 | Signed git tags for release | C-§8, Design Decisions §9 | — |
| I-45 | SBOM generation (`cargo-sbom` or equivalent) | C-§8, Design Decisions §9 | I-42 |
| I-46 | Binary checksums and release artifact process | C-§8, Design Decisions §9 | I-42 |

**Checkpoint:**
- Real Keychain path works on macOS for full `add` → `run` → relay flow
- Real `migrate` → `run` flow works against Claude Desktop config
- All C-§11 behavior gates satisfied
- Security claims documented and claim-consistent with contract
- Release artifacts include signed tags, SBOM, and checksums

---

## 4) Dependency Graph

This graph shows primary dependency chains. The task tables in §3 are the authoritative dependency source; edges omitted here for readability (e.g., I-06 → I-21, I-09 → I-30) are still real dependencies.

```
I-01 ──┬── I-02 ──┬── I-14 ─┬── I-16                    (M3: add/remove)
       │          │  I-15 ─┤
       │          │         └── I-17
       │          │
       │          └── I-37 ──┬── I-38                    (M6: doctor)
       │                     └── I-39
       │
       ├── I-03, I-04                                     (M1: CI, licenses)
       │
       ├── I-05 ──┬── I-11
       │          ├── I-14, I-15
       │          └── I-30
       │
       ├── I-06 ──┬── I-07 ──┬── I-17, I-26, I-36, I-39
       │          │          │
       │          └── I-08 ── I-40
       │
       ├── I-09 ──┬── I-10 ──┬── I-21
       │          │          └── I-37
       │          └── I-32
       │
       ├── I-12 ── I-30
       │
       ├── I-13 ── I-21
       │
       └── I-18 ── I-19 ── I-20 ──┬── I-21 ─┬── I-27   (M4: run)
                                   │         │
                                   └── I-26 ─┘
                                   │
                                   I-21 ── I-22 ── I-23
                                     │
                                     └── I-24, I-25
                                                  │
                                          ═══ GATE ═══
                                                  │
                                   I-28 ── I-29 ── I-30          (M5: migrate)
                                                     │
                                        I-31, I-32, I-33, I-35
                                                     │
                                                    I-34
                                                     │
                                                    I-36

I-08 ── I-40 ── I-41                                     (M7: integration)
I-42 ──┬── I-45
       └── I-46                                           (M7: release)
I-43, I-44                                                (M7: docs, tags)
```

Note on M5 graph: I-31, I-32, I-33, and I-35 all depend on I-30 directly (parallel). I-34 depends on I-31, I-32, and I-33 (not I-35). I-36 depends on I-30 through I-35. The vertical layout shows the convergence toward I-36, not a linear chain.

---

## 5) Critical Path

Two parallel paths merge at I-27 (integration test), both starting from I-20:

```
I-01 → I-18 → I-19 → I-20 ─┬── I-21 ─┬── I-27
                              └── I-26 ─┘
```

Both paths are 5 hops from I-01 to I-27. Neither gates the other — I-21 (run orchestration) and I-26 (relay proxy tests) can proceed in parallel once I-20 (carve-outs) is complete.

Parallelizable alongside these chains:
- I-05, I-06, I-07 (validate, secret trait, InMemory) — needed by I-21 and I-26
- I-09, I-10 (config model, config validation) — needed by I-21
- I-11 (VaultUri) — needed by I-21
- I-12, I-13 (registry, injector) — needed by I-21 and I-30
- I-14, I-15 (add/remove) — independent once I-02 + I-05 + I-06 exist

I-22/I-23 (process lifecycle, signal forwarding) and I-24/I-25 (verbose, errors) branch off I-21 but do not gate I-27. They must be complete before the gate checkpoint is satisfied but are not on the critical path to the integration test.

The gate blocker is the relay + transport chain. Foundation modules are parallel feeder work.

---

## 6) Traceability Summary

| Task Range | Milestone | Primary Contract Source |
|------------|-----------|----------------------|
| I-01 – I-04 | M1 Scaffold | C-§8, Design Decisions §9 |
| I-05 – I-13 | M2 Foundation | C-§4, C-§4.1, C-§4.3, C-§4.4, C-§7 |
| I-14 – I-17 | M3 add/remove | C-§5 `add`/`remove`, P-§5, P-§6 |
| I-18 – I-27 | M4 Runtime | C-§4.2, C-§5 `run`, C-§10, P-§4 |
| I-28 – I-36 | M5 Migration | C-§4, C-§5 `migrate`, P-§3 |
| I-37 – I-39 | M6 Doctor | C-§5 `doctor`, P-§7 |
| I-40 – I-46 | M7 Integration | C-§6, C-§11, Design Decisions §9 |

---

## 7) Test Plan Coverage Note

The secret-never-logged invariant (C-§6: "The relay MUST NOT log secret values at any verbosity level") is verified in I-26 by asserting no secret values appear in relay stderr output under any verbosity level. This provides baseline evidence during implementation. The Test and Verification Plan (§8.1 item 5) may define additional evidence requirements (e.g., audit of all log call sites).
