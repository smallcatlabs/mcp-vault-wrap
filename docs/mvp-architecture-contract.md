# mcp-vault-wrap MVP Architecture Contract

_Revision 9, March 2026_

---

## 1) Purpose and Role in the Document Stack

This document is the top-level MVP architecture contract for `mcp-vault-wrap`.

It defines:

- what MUST be true for MVP,
- what MUST NOT be done in MVP,
- and how downstream docs are derived from this contract.

This document is intentionally high-leverage and low-detail.  
This document MUST NOT prescribe implementation task breakdown, ownership, or timeline.  
It MAY define behavioral and evidence acceptance gates.  
Code-structure details belong in lower-level artifacts unless explicitly marked here as architectural seams.

---

## 2) MVP Outcome

The MVP MUST deliver a Rust CLI that:

- removes MCP credentials from plaintext Claude Desktop host configuration entries,
- stores those credentials in macOS Keychain,
- resolves credentials at runtime via `vault://` references, and
- relays MCP traffic transparently to downstream servers.

The MVP MUST be framed as credential-at-rest hardening for local stdio workflows.  
The MVP MUST NOT be framed as a complete MCP security solution.

---

## 3) Scope Contract

### In Scope (MVP)

- Host migration support: Claude Desktop only
- Platform: macOS only
- Secret backend: macOS Keychain only
- Profile model: `[profile.default]` namespace present; only `default` is supported in MVP
- Commands: `run`, `add`, `remove`, `migrate`, `doctor`
- Registry launch entries: `github` and `slack`
- Relay model: one-to-one (each `run` invocation proxies exactly one downstream server)

### Out of Scope (MVP)

- Cross-platform keystores
- Remote secret backends
- Docker local mode
- Remote TLS relay mode
- Prompt injection detection / response scanning
- Tool-definition pinning / rug-pull detection
- Secret lifecycle metadata (rotation, expiry)
- Multi-profile runtime support beyond `default`
- Multi-server aggregation behind a single relay endpoint

Out-of-scope items MAY be specified later in product-level documents.  
They MUST NOT be implicitly introduced through MVP implementation decisions.

---

## 4) Core Invariants

- **Fail-closed operations:** `run` and `migrate` MUST fail on security-critical ambiguity or missing prerequisites.
- **Deterministic classification:** migration MUST classify secrets using compiled registry definitions only; no heuristics.
- **Explicit intent:** migration MUST act only on servers explicitly passed via `--servers`, where each `--servers` value refers to a host config entry name.
- **Dual-name validation:** for each specified server name, migration MUST find both (a) a host config entry and (b) a registry definition with the same name; otherwise migration MUST fail with an actionable error.
- **Name safety invariant:** profile names, secret names, and migrated env-var names MUST match `[a-zA-Z0-9_.-]` at every input boundary.
- **Traceable safety path:** migration MUST provide `--dry-run`, backup protection, and actionable errors.
- **Thin host launcher:** post-migration host config MUST launch `mcp-vault-wrap run <server-name>` and MUST NOT retain migrated env vars (secret or non-secret) for migrated servers.
- **Non-secret preservation:** non-secret env vars for migrated servers MUST be preserved in relay TOML under `env`.
- **Server definition preservation:** migration MUST preserve the original server `command` and `args` from the host config entry in the relay TOML server definition. `run` MUST use these values to spawn the downstream server process.
- **Registry classification completeness:** migration MUST fail if the host config entry contains env vars not present in the registry definition for that server. The error MUST name the unrecognized variables. Registry entries define per-env-var classification (`secret` vs `config`) only; infrastructure-level env vars (proxy, TLS, runtime) are a post-MVP registry concern.
- **One-to-one relay model:** each `run <server-name>` invocation MUST proxy exactly one downstream server. The host config MUST launch a separate `mcp-vault-wrap run` process per migrated server.
- **Sampling expansion seam:** MVP design MUST preserve a host-side service handle seam so bidirectional sampling support can be added post-MVP without rearchitecture.

---

## 4.1) Required Architectural Seams (Day One)

- Secret access MUST be mediated through a `SecretBackend` abstraction.
- `SecretBackend` MUST expose an `authenticate()` method (no-op allowed for implicit-auth backends such as Keychain).
- MVP production secret backend MUST be macOS Keychain.
- MVP MUST include an `InMemoryBackend` for automated tests.

---

## 4.2) Relay Architecture (Day One)

- The relay MUST implement a protocol-level MCP proxy: an MCP server facade facing the host and an MCP client facing the downstream server.
- The relay MUST forward all MCP messages between host and downstream server by default.
- The relay MUST only intercept or modify messages for explicitly declared carve-outs. MVP carve-out: sampling capability MUST be filtered at capability negotiation.
- The MVP transport is stdio on both sides; the proxy architecture MUST support transport substitution on the host-facing side without relay-core changes.

---

## 4.3) Secret Reference Syntax and Keychain Naming

- Secret references in relay TOML MUST use the syntax `vault://<profile>/<secret-name>`.
- `run` MUST resolve these references through the `SecretBackend` at startup.
- Keychain entries MUST be stored with the service name `mcp-vault-wrap.<profile>.<secret-name>`.

---

## 4.4) Configuration Discovery and Versioning

- `run` MUST load relay configuration from `~/.config/mcp-vault-wrap/relay.toml` by default.
- This path MUST be overridable via `--config`.
- `migrate` MUST write to this same default path unless overridden.
- Relay TOML MUST include a top-level `config_version = 1` field. `run` MUST reject config files with an unrecognized version.

---

## 5) Command Behavior Contract

### `run <server-name>`

MUST:

- load relay config from the default path or `--config` override (see §4.4),
- resolve `vault://<profile>/<secret-name>` references from Keychain (see §4.3),
- spawn the downstream server using the `command` and `args` from the relay TOML server definition,
- relay MCP traffic as a protocol-level proxy per §4.2 (forward by default, intercept only for declared carve-outs).

MVP carve-out: MUST filter sampling capability from the client capabilities presented downstream, so unsupported sampling fails at capability negotiation time rather than mid-session.

### `add <profile> <secret-name>`

MUST validate name allowlist (`[a-zA-Z0-9_.-]`) and write secret to Keychain.

### `remove <profile> <secret-name>`

MUST validate name allowlist (`[a-zA-Z0-9_.-]`) and delete secret from Keychain.

### `migrate <host-config-path> --servers ...`

MUST execute ordered phases:

1. Validate
2. Write secrets
3. Write TOML
4. Rewrite host config

MUST be effectively all-or-nothing over the specified server set: all inputs are validated before any writes begin, and secret writes are idempotent so a rerun after partial failure converges to the correct state.
MUST treat each `--servers` argument as a literal host config entry name (no alias, fuzzy matching, command fingerprinting, or auto-discovery in MVP).  
MUST fail if relay TOML already exists (**MVP-only policy; revisit post-MVP**).  
MUST create and preserve a pre-migration backup.  
MUST enforce the same name allowlist (`[a-zA-Z0-9_.-]`) for migrated env var names and reject invalid names.
MUST preserve non-secret env vars for migrated servers in relay TOML `env`.
MUST preserve the original `command` and `args` from each host config entry in the relay TOML server definition.
MUST fail if a host config entry contains env vars not present in the registry definition for that server; the error MUST name the unrecognized variables.
MUST use re-run-all-phases recovery semantics: if partial TOML exists after failure/interruption, the user MUST remove it and rerun the same command.
MUST NOT implement phase-skipping or resume-state migration in MVP.

### `doctor`

MUST be diagnostic (not an operational gate).  
MUST report Keychain accessibility, secret reference presence, config validity, and permission issues.  
MUST NOT attempt downstream launchability checks in MVP.
MUST run only when explicitly invoked by the user.  
`run` and `migrate` MUST perform their own required validations and MUST NOT invoke `doctor` as a prerequisite step.

---

## 6) Security Model and Claims Versioning

### Protects Against

- plaintext credential scraping from host config files,
- accidental sharing/committing of static credential values in host config.

### Does Not Protect Against

- active same-user compromise in an unlocked session,
- prompt injection or tool-output attacks,
- runtime exposure characteristics of env-var injection,
- full host compromise where attacker can invoke the relay.

The relay MUST NOT log secret values at any verbosity level.

Release documentation MUST publish these boundaries verbatim or stricter.  
Release documentation MUST NOT expand claims beyond these boundaries without explicit model revision.

### Claims Versioning Rules

- Each release MUST include current "Protects Against" and "Does Not Protect Against" claims.
- If claims change, release notes MUST include a `Security Model Changes` section.
- Product/feature docs derived from this contract MUST remain claim-consistent with the release.

---

## 7) Registry Governance Contract (Solo-First, unchanged intent)

Registry changes are security-sensitive.

During solo-maintainer phase:

- a single maintainer MAY approve registry changes,
- but a registry checklist MUST be completed in the change record.

The checklist MUST include:

- per-variable classification rationale (`secret` vs `config`),
- migration impact note for existing users.

For initial/bootstrap registry entries with no prior users, migration impact MAY be recorded as `N/A (initial entry)`.

When multiple active maintainers exist, governance SHOULD transition to:

- two approvals, including one security-focused reviewer.

---

## 8) How to Build Out Downstream Artifacts

This section defines the required document stack and sequence.

### 8.1 Artifact hierarchy

1. **Architecture Contract** (this document)  
   Defines MVP boundaries and invariants.

2. **Product/Feature-Level Spec**  
   Defines user flows, command UX, error UX, and acceptance behavior per command.

3. **Technical Design Spec**  
   Defines internal modules, interfaces, data models, and migration/relay control flow.

4. **Implementation Plan**  
   Defines milestones, tasks, owners, ordering, and delivery checkpoints.

5. **Test and Verification Plan**  
   Defines unit/integration/manual/security tests and required evidence artifacts.

6. **Release Readiness Checklist**  
   Defines go/no-go requirements for packaging, claims, and release security posture.

### 8.2 Derivation rule

Each lower-level artifact MUST trace back to this contract.  
Any item that cannot be traced to a contract requirement, scope item, or deferred item MUST be rejected or explicitly proposed as a contract change.

---

## 9) Traceability Model (required)

To minimize drift and rework, use requirement IDs.

### 9.1 ID scheme

- `C-*` for contract requirements (this doc)
- `P-*` for product spec requirements
- `D-*` for technical design decisions
- `I-*` for implementation tasks
- `T-*` for test cases/evidence

### 9.2 Traceability requirements

- Every `P-*` MUST map to at least one `C-*`.
- Every `D-*` MUST map to at least one `P-*` and one `C-*`.
- Every `I-*` that implements functional behavior MUST map to at least one `D-*`. Infrastructure, release, and test-evidence tasks MUST trace to at least one `C-*` or `P-*`.
- Every `T-*` MUST map to behavior evidence for at least one `C-*` or `P-*`.

---

## 10) Stage Gates for Planning and Execution

The implementation plan MUST preserve tracer-bullet risk reduction:

- runtime path (`run`: resolve -> spawn -> relay) MUST be validated end-to-end before migration work is considered MVP-complete,
- migration implementation MUST NOT bypass this runtime-first gate.

### Gate A: Contract Stable

- MVP scope and invariants accepted
- open contract ambiguities resolved or deferred explicitly

### Gate B: Product Spec Ready

- command-level UX and error semantics specified
- migration behavior and dry-run output shape specified

### Gate C: Technical Design Ready

- module boundaries and interfaces defined
- fail-closed paths and error classification defined

### Gate D: Implementation Ready

- task plan sequenced and dependency-aware
- no task in scope lacks traceability links

### Gate E: Test Ready

- behavior and evidence tests defined for all acceptance gates
- fixture strategy and manual-test expectations documented

### Gate F: Release Ready

- security claims match contract
- checksums/signatures process complete
- known limitations are explicitly documented

---

## 11) Acceptance Gates (Behavior + Evidence)

MVP is complete only when both behavior and evidence gates are satisfied.

### Behavior Gates

- `migrate` works end-to-end for `github` and `slack` on Claude Desktop config.
- Migrated secrets are removed from host config and stored in Keychain.
- Non-secret env vars are preserved in relay TOML `env` map.
- `run` successfully resolves `vault://` references and relays MCP session traffic.
- `run` enforces sampling carve-out at capability negotiation.
- `doctor` produces actionable diagnostic output for missing secrets, invalid config, and permission issues.

### Evidence Gates

- migration fixtures cover success and failure paths (including existing TOML refusal, already-migrated entry, missing registry entry, unrecognized env vars),
- dry-run output is verified against fixture expectations,
- name validation tests cover allowlist acceptance/rejection boundaries,
- security model section exists in repository docs and matches this contract,
- release artifact process includes checksums and signature publication.

---

## 12) Assumptions and Constraints

- Host provides stdio server launch model compatible with relay launcher entries.
- Keychain interaction may require user approval in desktop session.
- Headless sessions may fail Keychain interaction and are not a supported primary MVP path.
- Runtime secret injection uses env vars in MVP and inherits known runtime visibility tradeoffs.

---

## 13) Deferred from MVP Contract

- incremental migration/merge semantics for existing TOML,
- multi-host parser expansion strategy,
- non-env secret injection mechanisms,
- remote/deployment transport modes,
- infrastructure-level env var classification (proxy, TLS, runtime vars),
- expanded governance model for external contributors.

These items SHOULD be handled in post-MVP product and governance documents.

