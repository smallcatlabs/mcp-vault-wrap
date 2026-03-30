# mcp-vault-wrap Design Decisions

Decisions made during design review, March 2026. These resolve the open questions identified in the design report and establish constraints for MVP implementation.

## 1. Endpoint Topology: Separate Endpoints

The relay presents **separate endpoints** — one relay instance per backend MCP server. The host sees N relay entries in its config, each proxying one downstream server.

**Rationale:** Simpler to build, cleaner security boundaries, no tool name deconfliction needed. Aggregation can be layered on top later; splitting an aggregated design apart would be harder.

**Trade-off acknowledged:** Host config is not simplified — entries are redirected, not consolidated. The migration story is weaker than it would be with aggregation.

## 2. Config Format & Secret Syntax: TOML + `vault://` URIs

The relay's configuration uses **TOML**. Secret references use **URI-style placeholders** with the `vault://` scheme.

Example:
```toml
[profile.default.servers.github]
command = "mcp-server-github"
secret_env.GITHUB_TOKEN = "vault://default/GITHUB_TOKEN"
```

**Rationale:**
- TOML aligns with Rust ecosystem conventions, supports comments, and avoids YAML's implicit type coercion footguns.
- URI syntax is self-describing and unambiguous in any config format.
- The URI identifies profile and secret name only. The secret backend for a given profile is determined by relay configuration, not by the URI. This keeps backend selection as a single config-level decision rather than requiring per-reference changes when switching backends (e.g., Keychain to 1Password, or macOS to Linux).

## 3. Failure Policy: Fail-Closed, Loud Errors, No Silent Fallback

Every failure is a hard stop with a clear, actionable error message. No silent degradation, no automatic retry, no fallback to insecure behavior.

Specific scenarios:
- **Secret not found:** Server does not start. Error names the missing secret and tells the user how to add it.
- **Keychain locked/unavailable:** Server does not start. Error explains the keychain state.
- **Backend server fails to start:** Report failure clearly, stop. No retry loop.
- **Relay crashes mid-session:** Host sees the server as unavailable. No clever recovery in MVP.

The `migrate` command **preserves a backup** of the original plaintext config. If the relay is broken, the user can revert manually. This is the "break glass" mechanism — not a built-in bypass.

## 4. Injection Mechanism: Trait Abstraction, Env-Var MVP

Secret injection is modeled as a **trait/interface** (`Injector`) from day one, even though MVP only implements environment variable injection.

**MVP implementation:** Secrets are passed as environment variables when spawning the server child process.

**Known limitation:** Env vars are visible in `/proc/<pid>/environ` (Linux) and via `ps` (macOS). The exposure window is shorter than files on disk but not zero. This limitation is documented honestly.

**Planned second implementation:** Tmpfs file injection — write secret to a temporary in-memory file, pass the path to the server, delete on exit. This covers servers that expect config files rather than env vars.

## 5. Config Security: File Permission Checks at Startup

The relay checks file permissions on its config at startup, **SSH-style**:
- Config file should be `600` (owner read/write only).
- If permissions are too open, the relay warns or refuses to start.

**Deferred:** Cryptographic signing, checksumming with a separate trust anchor, config-change detection via stored hashes. These add complexity that isn't justified for a single local user in MVP. Stronger config protection is appropriate when remote mode or multi-user scenarios arrive.

## 6. Multi-Identity: Profile Nesting from Day One

The TOML config uses **`[profile.default]`** as a top-level namespace from day one. MVP only reads and supports `default`.

**Rationale:** Adding profile support later without this nesting would be a breaking config change. The cost of the extra nesting now is near zero. Future profiles (`[profile.work]`, `[profile.client]`) slot in naturally, selectable via `--profile` flag or `MCP_VAULT_PROFILE` env var.

The `vault://` URI scheme already supports this: `vault://default/GITHUB_TOKEN` vs `vault://work/GITHUB_TOKEN`.

## 7. Secret Lifecycle: Bare URIs, No Metadata

Secrets are referenced as **bare `vault://` URI strings** in the config. No structured metadata (expiration dates, rotation policies) in MVP.

**Rationale:** Lifecycle metadata is only useful if there's a reliable way to populate it. The macOS Keychain doesn't store expiration dates for generic passwords. Asking users to manually enter expiration dates defeats the purpose.

When a future backend (1Password CLI, remote vault) natively provides lifecycle metadata, the config format can evolve to structured secret objects at that point.

For MVP, if a secret is expired, the downstream server returns an auth error, and the relay surfaces that clearly.

## 8. Logging: Operations Not Payloads

**Default logging:** Stderr, errors only. Logs record operations ("server started," "tool called," "server stopped") but **never** arguments, responses, or secret values.

**`--verbose` flag:** Enables detailed logging of tool call metadata. Carries a visible warning at startup: verbose mode may log sensitive metadata.

**Deferred:** Audit logging, structured log output, log redaction engine. These arrive with the observability features in Level 4 of the roadmap.

## 9. Supply Chain Security: Three Tiers

### From day one:
- All dependencies pinned via committed `Cargo.lock` (the authoritative pin for this binary crate; `Cargo.toml` uses standard semver ranges)
- CI pipeline: build, test, `cargo audit` on every PR
- Signed git tags for releases

### Before first public release:
- Reproducible builds via pinned Rust toolchain (`rust-toolchain.toml`)
- Binary checksums published alongside releases
- SBOM generation (`cargo-sbom` or equivalent)

### Deferred:
- Sigstore/cosign binary signing
- Auto-update mechanism — **explicitly rejected**. Users must opt into every version change. Distribution via package managers (Homebrew, `cargo install`).

**Principle:** No auto-updates, ever. For a security tool that handles secrets, the user must explicitly choose every version change.

## 10. Doctor Command: Explicit Diagnostic Only

The `doctor` command runs **only when explicitly invoked** — no automatic preflight before every `wrap` command.

Checks for MVP:
- **Keychain accessible:** Can the relay reach the macOS Keychain?
- **Secrets present:** Does each secret referenced in the config exist in the Keychain?
- **Config valid:** Does the TOML parse? Are `vault://` URIs well-formed? Are file permissions correct?

Downstream server launchability checks (binary exists, is executable) are **deferred from MVP** — they are brittle across server types (npx, uvx, docker, native binaries) and not worth the implementation cost.

Each check outputs pass/fail with actionable guidance:
```
 Keychain accessible
 Secret "GITHUB_TOKEN" not found in keychain
   Run: mcp-vault-wrap add default/GITHUB_TOKEN
 Config valid (/Users/tom/.config/mcp-vault-wrap/relay.toml)
```

**Rationale:** Fail-closed behavior already stops execution with a clear error on every run. `doctor` is for comprehensive diagnosis, not routine preflight.
