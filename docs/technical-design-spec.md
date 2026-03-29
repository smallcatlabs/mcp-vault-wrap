# mcp-vault-wrap Technical Design Spec

_March 2026 — Derived from MVP Architecture Contract (Revision 8) and Product Spec_

---

## 1) Purpose

This document defines the internal module boundaries, interfaces, data structures, and control flow for the MVP implementation. Every decision traces to the architecture contract (C-\*) and product spec (P-\*) via D-\* identifiers.

---

## 2) Module Map

```
mcp-vault-wrap/
├── src/
│   ├── main.rs              # Entry point, CLI dispatch
│   ├── cli.rs               # Argument parsing, validation, --version
│   ├── registry.rs          # Compiled server definitions
│   ├── secret/
│   │   ├── mod.rs           # SecretBackend trait
│   │   ├── keychain.rs      # macOS Keychain implementation
│   │   └── memory.rs        # InMemoryBackend (tests, future MVP+1)
│   ├── inject/
│   │   ├── mod.rs           # Injector trait
│   │   └── env.rs           # EnvInjector (env-var injection)
│   ├── config/
│   │   ├── mod.rs           # RelayConfig data model + TOML serde
│   │   └── validate.rs      # Config-level validation (version, permissions)
│   ├── host/
│   │   ├── mod.rs           # HostConfig trait
│   │   └── claude_desktop.rs# Claude Desktop JSON parser
│   ├── migrate.rs           # Migration orchestration (4 phases)
│   ├── relay/
│   │   ├── mod.rs           # Proxy loop (raw passthrough + carve-outs)
│   │   └── carveouts.rs     # initialize message filtering
│   ├── transport/
│   │   ├── mod.rs           # Transport trait (async)
│   │   └── stdio.rs         # Newline-delimited JSON over stdio
│   └── validate.rs          # Shared name validation
```

### Module Dependency Rules

- `relay` depends on `transport` (trait only), `config` (data model).
- `migrate` depends on `host` (trait only), `registry`, `secret` (trait only), `config`.
- `run` orchestration (in `cli` or `main`) depends on `secret` (trait only), `inject` (trait only), `config`, `relay`, `transport`.
- `cli` wires concrete implementations together.
- `registry`, `validate`, `inject`, and `transport` depend on nothing else in the crate.

---

## 3) Design Decisions Index

| ID | Decision | Traces To |
|----|----------|-----------|
| D-01 | Raw JSON-RPC passthrough with selective parsing | C-§4.2, P-§4 |
| D-02 | Async Transport trait owns message framing | C-§4.2 |
| D-03 | Hardcoded carve-out for sampling filter | C-§4.2, C-§5 `run` |
| D-04 | Validate-first atomicity with idempotent secret writes | C-§4, C-§5 `migrate` |
| D-05 | SecretBackend trait with authenticate/get/set/delete/exists | C-§4.1 |
| D-06 | Separate `secret_env` and `env` maps in relay TOML | C-§4.3, C-§4.4 |
| D-07 | Light host parser trait (parse + write) | C-§3, C-§13 |
| D-08 | Static Rust data literals for compiled registry | C-§4, C-§7 |
| D-09 | Injector trait with EnvInjector MVP implementation | Design Decisions §4 |
| D-10 | Host-side handle seam for future sampling support | C-§4 sampling expansion seam |
| D-11 | Raw passthrough for malformed/unknown messages | C-§4.2 (forward by default) |

---

## 4) Interfaces

### 4.1 SecretBackend Trait — D-05

```rust
pub trait SecretBackend {
    fn authenticate(&self) -> Result<(), SecretError>;
    fn get(&self, profile: &str, name: &str) -> Result<String, SecretError>;
    fn set(&self, profile: &str, name: &str, value: &str) -> Result<(), SecretError>;
    fn delete(&self, profile: &str, name: &str) -> Result<(), SecretError>;
    fn exists(&self, profile: &str, name: &str) -> Result<bool, SecretError>;
}
```

**KeychainBackend:** Maps `(profile, name)` to Keychain service name `mcp-vault-wrap.<profile>.<name>`. Account field is the literal string `mcp-vault-wrap`. `authenticate()` is a no-op (Keychain auth is implicit per session). [C-§4.1, C-§4.3]

**InMemoryBackend:** `HashMap<(String, String), String>`. `authenticate()` is a no-op. Used for automated tests in MVP; becomes the production backend in MVP+1 and MVP+2. [C-§4.1]

`set()` is idempotent — overwrites if the entry already exists. This supports the migration recovery path where a rerun writes the same secrets again. [D-04]

### 4.2 Transport Trait — D-02

```rust
#[async_trait]
pub trait Transport {
    async fn recv(&mut self) -> Result<Vec<u8>, TransportError>;
    async fn send(&mut self, message: &[u8]) -> Result<(), TransportError>;
}
```

The trait is async to support the relay's `tokio::select!` proxy loop, which must concurrently await messages from both the host and downstream sides. This is a day-one decision that avoids rework at MVP+1 when the transport changes from stdio to Unix sockets — both implement the same async trait, and the relay core does not change. [C-§4.2]

Each `recv()` returns one complete JSON-RPC message as raw bytes. Each `send()` writes one complete message. The transport owns framing (newline-delimited JSON for stdio). The relay core never deals with partial reads or message boundary detection.

**StdioTransport:** Wraps a pair of async streams (read + write) using `tokio::io::BufReader` for line-based reading. Two instances per relay: one for the host-facing side (relay's own stdin/stdout), one for the downstream side (child process stdin/stdout). [C-§4.2]

**Note:** Downstream server stderr is NOT part of the Transport abstraction. Stderr is handled at the process lifecycle level via `Stdio::inherit()` (see §9). The Transport trait models only the MCP message channel.

### 4.3 Injector Trait — D-09

```rust
pub trait Injector {
    fn inject(
        &self,
        secrets: HashMap<String, String>,
        command: &mut Command,
    ) -> Result<(), InjectError>;
}
```

**EnvInjector:** Sets secrets as environment variables on the child process Command. This is the only MVP implementation.

```rust
pub struct EnvInjector;

impl Injector for EnvInjector {
    fn inject(
        &self,
        secrets: HashMap<String, String>,
        command: &mut Command,
    ) -> Result<(), InjectError> {
        command.envs(secrets);
        Ok(())
    }
}
```

The trait exists because tmpfs file injection (write secret to a temporary in-memory file, pass the path as an env var, delete on exit) is a planned post-MVP implementation for servers that expect config files rather than env vars. [Design Decisions §4]

### 4.4 HostConfig Trait — D-07

```rust
pub trait HostConfig {
    fn parse(path: &Path) -> Result<HostConfigData, HostConfigError>;
    fn write(path: &Path, data: &HostConfigData) -> Result<(), HostConfigError>;
}

pub struct HostConfigData {
    pub servers: HashMap<String, HostServerEntry>,
    pub raw: serde_json::Value,  // preserves non-server fields for round-trip
}

pub struct HostServerEntry {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}
```

`raw` preserves the full original JSON structure. `write()` updates only the `mcpServers` entries for migrated servers, leaving everything else untouched. This prevents migration from clobbering unrelated host config fields. [C-§5 `migrate`]

**ClaudeDesktopConfig:** Reads `~/Library/Application Support/Claude/claude_desktop_config.json`. The path is derived from the `--host claude-desktop` flag, not hardcoded in the trait. [C-§3, P-§3]

---

## 5) Data Structures

### 5.1 Relay TOML Model — D-06

```rust
pub struct RelayConfig {
    pub config_version: u32,
    pub profile: HashMap<String, ProfileConfig>,
}

pub struct ProfileConfig {
    pub servers: HashMap<String, ServerConfig>,
}

pub struct ServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub secret_env: HashMap<String, String>,  // values are vault:// URIs
    pub env: HashMap<String, String>,          // plaintext non-secret values
}
```

Serialized TOML shape:

```toml
config_version = 1

[profile.default.servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
secret_env.GITHUB_TOKEN = "vault://default/GITHUB_TOKEN"

[profile.default.servers.slack]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-slack"]
secret_env.SLACK_BOT_TOKEN = "vault://default/SLACK_BOT_TOKEN"
env.SLACK_TEAM_ID = "T01234567"
```

**Validation at load time:** `config_version` must equal `1`. All `secret_env` values must parse as valid `vault://` URIs. All `env` values must NOT start with `vault://`. File permissions must be `600`. [C-§4.4, P-§4]

### 5.2 Vault URI — D-01, D-06

```rust
pub struct VaultUri {
    pub profile: String,
    pub secret_name: String,
}
```

Parsed from the string `vault://<profile>/<secret-name>`. Both components must pass name validation. No other URI components (scheme is always `vault`, no host, no port, no query). [C-§4.3]

### 5.3 Registry Entry — D-08

```rust
pub enum EnvClassification {
    Secret,
    Config,
}

pub struct RegistryEntry {
    pub name: &'static str,
    pub env_vars: &'static [(&'static str, EnvClassification)],
}
```

MVP registry:

```rust
pub fn registry() -> &'static [RegistryEntry] {
    &[
        RegistryEntry {
            name: "github",
            env_vars: &[
                ("GITHUB_TOKEN", EnvClassification::Secret),
            ],
        },
        RegistryEntry {
            name: "slack",
            env_vars: &[
                ("SLACK_BOT_TOKEN", EnvClassification::Secret),
                ("SLACK_TEAM_ID", EnvClassification::Config),
            ],
        },
    ]
}
```

Registry lookup is by exact name match against the host config entry name. [C-§4, C-§7]

### 5.4 Name Validation — shared

```rust
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}
```

Applied at every input boundary: CLI arguments, env var names during migration, profile and secret names. [C-§4]

---

## 6) Control Flow

### 6.1 `run <server-name>` — P-§4

MVP `run` always uses `profile.default`. Other profiles in the TOML are ignored. A `--profile` flag and `MCP_VAULT_PROFILE` env var are reserved for post-MVP. [C-§3]

```
1. Load relay TOML from default path or --config override
2. Validate config_version == 1
3. Validate file permissions == 600
4. Look up server-name in profile.default.servers
   -> fail if not found
5. Parse all vault:// URIs in secret_env
6. Call backend.authenticate()
7. For each vault URI, call backend.get(profile, name)
   -> fail on first missing secret
8. Build Command from server definition (command + args)
9. Call injector.inject(resolved_secrets, &mut command) [D-09]
10. Set non-secret env vars on command (.envs(server.env))
11. Spawn downstream server
    -> fail if spawn fails
12. Wire transport:
    - host_transport = StdioTransport(relay stdin, relay stdout)
    - downstream_transport = StdioTransport(child stdout, child stdin)
    - Store Arc handle to host_transport's send side [D-10]
13. Enter proxy loop:
    - Select on: host_transport.recv() and downstream_transport.recv()
    - On message from host:
      - If first message: parse as initialize request, apply sampling
        carve-out (strip params.capabilities.sampling), re-serialize,
        forward to downstream [D-03]
      - Otherwise: forward raw bytes to downstream_transport
    - On message from downstream:
      - Forward raw bytes to host_transport
    - On transport recv() error: log to stderr if --verbose, shut down
      relay and exit [D-11]
    - On downstream process exit: exit with same code
14. Pass downstream stderr through to relay stderr (via Stdio::inherit,
    not Transport — see §4.2)
```

**Sampling carve-out (D-03):** The `initialize` request is the first message in the MCP protocol, sent from the host (client) to the relay. The relay parses this request, strips `params.capabilities.sampling` if present to prevent the downstream server from attempting sampling callbacks, re-serializes, and forwards to the downstream server. The `initialize` response from the downstream server is forwarded unmodified. All subsequent messages in both directions are unmodified passthrough. [C-§4.2, C-§5 `run`]

### 6.2 `migrate --host <host> --servers <names>` — P-§3

```
Phase 1: Validate
  1. Resolve host config path from --host flag
     -> fail if unsupported host
  2. Parse host config via HostConfig::parse()
  3. For each server name in --servers:
     a. Verify server exists in host config
        -> fail with available servers list if not found
     b. Verify server exists in registry
        -> fail with supported servers list if not found
     c. Validate all env var names in host config entry
        -> fail if any name is invalid
     d. Verify every env var in host config has a classification in registry
        -> fail naming unrecognized variables if any are unclassified
  4. Verify relay TOML does not already exist at target path
     -> fail if exists
  5. Verify Keychain is accessible (backend.authenticate())

Phase 2: Write secrets
  For each server, for each env var classified as Secret:
    backend.set(profile, env_var_name, env_var_value)
    set() is idempotent — safe on rerun [D-04]

Phase 3: Write relay TOML
  1. Create config directory (~/.config/mcp-vault-wrap/) if it does not exist
  2. Build RelayConfig from registry classification:
     - Secret vars -> secret_env with vault:// URIs
     - Config vars -> env with plaintext values
     - command + args preserved from host config entry
  3. Write to target path with permissions 600

Phase 4: Rewrite host config
  1. Create backup: <original-path>.bak
     -> fail if .bak already exists (do not overwrite a previous backup)
  2. For each migrated server, replace the host config entry:
     - command -> "mcp-vault-wrap"
     - args -> ["run", "<server-name>"]
     - env -> removed entirely (secrets are in Keychain, non-secrets are in TOML)
  3. Write modified host config via HostConfig::write()
```

**Dry-run mode:** Executes Phase 1 validation fully (including Keychain accessibility check). Phases 2-4 print what would happen in the format specified by P-§3 but write nothing. [C-§5 `migrate`, P-§3]

**Recovery path (D-04):**
- Phase 2 failure: Rerun same command. `set()` overwrites existing secrets harmlessly.
- Phase 3 failure: Delete partial TOML, rerun.
- Phase 4 failure: Restore host config from `.bak`, delete TOML, rerun.

### 6.3 `add <profile> <secret-name>` — P-§5

```
1. Validate profile name and secret name
2. Read secret value from stdin (interactive prompt or pipe)
3. If --force not set: check backend.exists(profile, name)
   -> fail if exists
4. backend.set(profile, name, value)
5. Print confirmation
```

### 6.4 `remove <profile> <secret-name>` — P-§6

```
1. Validate profile name and secret name
2. Check backend.exists(profile, name)
   -> fail if not found
3. backend.delete(profile, name)
4. Print confirmation
```

### 6.5 `doctor` — P-§7

```
1. Check relay TOML exists at default or --config path
   -> report missing and stop if not found
2. Parse and validate config (version, structure)
3. Check file permissions
4. Check Keychain accessibility (backend.authenticate())
   -> if inaccessible, report and skip secret checks
5. For each server in profile.default:
   For each secret_env entry:
     Check backend.exists(profile, secret_name)
     -> report pass/fail per secret
   For each env entry:
     Report present (informational)
6. Print summary: all passed or N issues found
7. Exit 0 if all pass, non-zero if any issue
```

---

## 7) Error Classification

Errors fall into three categories that determine presentation:

| Category | Behavior | Example |
|----------|----------|---------|
| **User-actionable** | Print clear message to stderr with fix instructions, exit non-zero | Missing secret, bad permissions, unsupported host |
| **Internal** | Print message to stderr, exit non-zero | TOML serialization failure, unexpected Keychain API error |
| **Protocol (D-11)** | Log to stderr if --verbose, shut down relay | Transport recv error, malformed framing |

For messages that are valid framing but contain unexpected/unknown JSON-RPC content: the relay forwards them as-is per the passthrough design (D-01). The relay does not validate MCP message contents beyond the sampling carve-out. Malformed or unknown messages are the host's and downstream server's problem, not the relay's. [C-§4.2]

Each module defines its own error enum. The CLI layer maps module errors to user-facing messages matching the product spec's prescribed error strings. [P-§3-§7]

---

## 8) Relay Proxy Detail — D-01, D-02, D-03, D-10

### Message Flow

```
Host (Claude Desktop)           Relay                    Downstream Server
        |                         |                            |
        |-- stdin JSON-RPC ------>|                            |
        |                    [recv from host]                  |
        |                    [carve-out check]                 |
        |                    [send to downstream]              |
        |                         |-- stdin JSON-RPC --------->|
        |                         |                            |
        |                         |<-- stdout JSON-RPC --------|
        |                    [recv from downstream]            |
        |                    [send to host]                    |
        |<-- stdout JSON-RPC -----|                            |
        |                         |                            |
        |                         |<-- stderr ----------------|
        |                         |-- stderr (passthrough) --> (relay stderr)
```

### Proxy Loop Structure

The proxy loop uses `tokio::select!` over two async transports. The relay proxies one connection, so a single-task select loop is sufficient. Both `recv()` calls are async (D-02), so they compose naturally in the select without blocking threads.

Message ordering is preserved within each direction: messages from the host are forwarded to the downstream in the order received, and vice versa. The single-task select loop guarantees this — there is no concurrent dispatch that could reorder messages within a direction.

### Carve-Out Application Point

The sampling carve-out fires exactly once: on the first host-to-downstream message (the `initialize` request from the host/client). The relay:

1. Parses the raw bytes as JSON
2. Checks for `params.capabilities.sampling`
3. If present, removes the key and re-serializes
4. Forwards the (possibly modified) bytes

A boolean flag tracks whether the first message has been processed. After that, all messages flow through unmodified. [C-§4.2]

### Host-Side Handle Seam — D-10

At relay setup (step 12 of the `run` flow), the send side of the host-facing transport is wrapped in an `Arc` and stored alongside the proxy loop state. In MVP, only the proxy loop uses this handle to forward messages to the host.

Post-MVP, this handle enables bidirectional sampling support: when the downstream server sends a `sampling/createMessage` request, a sampling handler can use the `Arc` handle to forward the request to the host, await the host's response, and return it to the downstream server. This reverse-direction flow is the capability that the sampling carve-out currently blocks. The `Arc` handle is the seam that allows it to be unblocked without restructuring the proxy loop. [C-§4 sampling expansion seam]

---

## 9) Downstream Process Lifecycle

```
Spawn:
  Command::new(command).args(args)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit())    // stderr passthrough, outside Transport

Running:
  Relay owns child stdin (write) and child stdout (read)
  Child stderr goes directly to relay stderr (inherited, not via Transport)

Shutdown:
  - If host closes stdin (EOF): relay closes child stdin, waits for child exit
  - If child exits: relay reads remaining stdout, then exits with child's exit code
  - If relay receives SIGTERM/SIGINT: forward signal to child, wait for exit
```

Note: secret injection happens between Command construction and spawn, via the Injector trait (D-09). Non-secret env vars from the TOML `env` map are set directly on the Command. The spawn step receives a fully-configured Command.

Signal forwarding ensures the downstream server gets a clean shutdown opportunity rather than being orphaned. [P-§4]

---

## 10) File Permissions and Security

- Relay TOML is written with mode `600` by `migrate`. [C-§5 `migrate`]
- Config directory (`~/.config/mcp-vault-wrap/`) is created with mode `700` if it does not exist.
- `run` checks TOML permissions at startup and refuses to proceed if not `600`. [P-§4]
- Secret values are never logged at any verbosity level. [C-§6]
- `--verbose` logs operation names and metadata only. [Design Decisions §8]
- `doctor` uses `exists()` to check secrets, never `get()` — secret values are not loaded into memory during diagnostics. [D-05]

---

## 11) Crate Dependency Expectations

These are initial expectations, not locked choices. Final crate selection happens during implementation.

| Purpose | Likely Crate | Notes |
|---------|-------------|-------|
| CLI parsing | `clap` | Derive mode for subcommands, `--version` built-in |
| TOML serde | `serde` + `toml` | Standard Rust ecosystem |
| JSON parsing | `serde_json` | Host config + JSON-RPC carve-out parsing |
| Keychain access | `security-framework` | Rust bindings to macOS Security.framework |
| Async runtime | `tokio` | select loop for bidirectional proxy |
| Process spawning | `tokio::process` | Async child process with piped stdio |

---

## 12) Testing Strategy Implications

The trait boundaries (SecretBackend, Transport, HostConfig, Injector) are the primary test seams:

- **Relay proxy tests:** `InMemoryBackend` + mock `Transport` that feeds canned JSON-RPC messages. Verify passthrough and sampling carve-out.
- **Migration tests:** `InMemoryBackend` + mock `HostConfig` (in-memory JSON). Verify all four phases, failure cases, idempotent rerun, and backup collision.
- **Config tests:** Round-trip TOML serialization, vault URI parsing, permission validation.
- **Registry tests:** Classification correctness, completeness checks, unknown-env-var rejection.
- **Injector tests:** Verify `EnvInjector` sets env vars on Command.

Full test plan is a separate artifact (§8.1 item 5). [C-§8.1]

---

## 13) Traceability Summary

| Design Decision | Contract Source | Product Spec Source |
|----------------|----------------|-------------------|
| D-01 Raw JSON-RPC passthrough | C-§4.2 | P-§4 Protocol Notes |
| D-02 Async Transport trait owns framing | C-§4.2 | — |
| D-03 Hardcoded sampling carve-out | C-§4.2, C-§5 `run` | P-§4 Protocol Notes |
| D-04 Validate-first + idempotent writes | C-§4, C-§5 `migrate` | P-§3 Behavior Notes |
| D-05 SecretBackend trait shape | C-§4.1 | — |
| D-06 Separate secret_env / env maps | C-§4.3, C-§4.4 | P-§3 Success Output |
| D-07 Light HostConfig trait | C-§3, C-§13 | P-§3 |
| D-08 Static registry data | C-§4, C-§7 | — |
| D-09 Injector trait + EnvInjector | Design Decisions §4 | — |
| D-10 Host-side handle seam | C-§4 sampling expansion seam | — |
| D-11 Raw passthrough for unknown messages | C-§4.2 | — |
