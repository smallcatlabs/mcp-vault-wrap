# mcp-vault-wrap Product Specification

_March 2026 — Derived from MVP Architecture Contract (Revision 8)_

---

## 1) Purpose

This document defines user-facing behavior for each MVP command: invocation syntax, output on success, output on failure, and edge-case handling. Every requirement traces to the architecture contract.

---

## 2) Global Conventions

- All errors go to **stderr** and result in a non-zero exit code.
- All commands fail with an actionable error rather than guessing or falling back silently.
- Name validation applies everywhere: profile names, secret names, and env var names MUST match `[a-zA-Z0-9_.-]`.
- Config path defaults to `~/.config/mcp-vault-wrap/relay.toml`, overridable via `--config` on commands that read or write config.
- `--version` prints the version and exits.

---

## 3) `migrate`

### Invocation

```bash
mcp-vault-wrap migrate --host claude-desktop --servers github,slack
```

**Required flags:**

- `--host <host-name>` — identifies the MCP host. MVP supports `claude-desktop` only. Maps internally to the host's known config path.
- `--servers <name,...>` — comma-separated list of host config entry names to migrate.

**Optional flags:**

- `--dry-run` — preview all changes without writing anything.
- `--config <path>` — override the default relay TOML output path.

### Success Output

```
Backing up ~/Library/Application Support/Claude/claude_desktop_config.json → claude_desktop_config.json.bak
Migrating server: github
  GITHUB_TOKEN → Keychain (mcp-vault-wrap.default.GITHUB_TOKEN)
Migrating server: slack
  SLACK_BOT_TOKEN → Keychain (mcp-vault-wrap.default.SLACK_BOT_TOKEN)
  SLACK_TEAM_ID → relay.toml env (non-secret)
Wrote relay config → ~/.config/mcp-vault-wrap/relay.toml
Updated ~/Library/Application Support/Claude/claude_desktop_config.json (2 servers migrated)
```

### Dry-Run Output

```
[dry-run] Backing up ~/Library/Application Support/Claude/claude_desktop_config.json → claude_desktop_config.json.bak
[dry-run] Migrating server: github
  GITHUB_TOKEN → Keychain (mcp-vault-wrap.default.GITHUB_TOKEN)
[dry-run] Migrating server: slack
  SLACK_BOT_TOKEN → Keychain (mcp-vault-wrap.default.SLACK_BOT_TOKEN)
  SLACK_TEAM_ID → relay.toml env (non-secret)
[dry-run] Would write relay config → ~/.config/mcp-vault-wrap/relay.toml
[dry-run] Would update ~/Library/Application Support/Claude/claude_desktop_config.json (2 servers migrated)
No changes made.
```

### Failure Cases

**Server name not found in host config:**
```
Error: Server "github" not found in claude_desktop_config.json
Available servers: slack, postgres, filesystem
```

**Server name not found in registry:**
```
Error: No registry definition for server "custom-server"
Supported servers: github, slack
```

**Unrecognized env var in host config:**
```
Error: Server "github" has unrecognized environment variables: GITHUB_ENTERPRISE_URL
All env vars must be defined in the registry for classification.
```

**Relay TOML already exists:**
```
Error: Relay config already exists at ~/.config/mcp-vault-wrap/relay.toml
Remove it and rerun to migrate from scratch.
```

**Invalid env var name:**
```
Error: Server "github" has invalid env var name: "MY KEY"
Names must match [a-zA-Z0-9_.-]
```

**Keychain write failure:**
```
Error: Failed to write secret "GITHUB_TOKEN" to Keychain: access denied
Ensure mcp-vault-wrap has Keychain access in System Settings → Privacy & Security.
```

**Unsupported host:**
```
Error: Unsupported host "cursor"
Supported hosts: claude-desktop
```

### Behavior Notes

- All inputs are validated before any writes begin. If a write phase fails after validation, the system may be in a partial state: secrets written to Keychain are idempotent (safe to overwrite on rerun), and the host config backup enables recovery. See "Recovery" below.
- A backup of the original host config is created before any modifications.
- Each `--servers` value is matched as a literal host config entry name. No fuzzy matching or auto-discovery.
- The original server `command` and `args` are preserved in the relay TOML server definition.
- Secret vs non-secret classification is determined entirely by the compiled registry definition for that server.

### Recovery

If migration fails or is interrupted during write phases:

- **Secrets already written to Keychain** are harmless — rerunning overwrites them with the same values.
- **Partial relay TOML** must be deleted before rerunning.
- **Host config** can be restored from the `.bak` backup if it was partially modified.

The recovery procedure is always: restore host config from backup if needed, delete relay TOML if it exists, rerun the same `migrate` command.

---

## 4) `run`

### Invocation

```bash
mcp-vault-wrap run <server-name>
```

Typically launched by the MCP host, not the user directly. The host config is rewritten by `migrate` to invoke this command.

**Optional flags:**

- `--config <path>` — override the default relay TOML path.
- `--verbose` — emit relay startup and operational messages to stderr.

### Behavior

1. Load relay TOML and validate config version.
2. Look up the named server definition.
3. Resolve all `vault://<profile>/<secret-name>` references from Keychain.
4. Spawn the downstream server using the `command` and `args` from the TOML definition, with resolved secrets and non-secret env vars in the process environment.
5. Proxy MCP messages between stdin/stdout (host-facing) and the downstream server's stdin/stdout.
6. Pass downstream server's stderr through to the relay's stderr.
7. When the downstream server exits, the relay exits with the same exit code.

### Output

**Default (silent):** No relay output on stderr. Downstream server's stderr passes through.

**With `--verbose`:**
```
[mcp-vault-wrap] Config: ~/.config/mcp-vault-wrap/relay.toml (version 1)
[mcp-vault-wrap] Resolved 1 secret for server "github"
[mcp-vault-wrap] Starting: npx @modelcontextprotocol/server-github
```

### Failure Cases

**Server not found in relay TOML:**
```
Error: Server "github" not defined in ~/.config/mcp-vault-wrap/relay.toml
Run "mcp-vault-wrap doctor" to diagnose configuration issues.
```

**Relay TOML not found:**
```
Error: Relay config not found at ~/.config/mcp-vault-wrap/relay.toml
Run "mcp-vault-wrap migrate --host claude-desktop --servers ..." to set up.
```

**Config file permissions too open:**
```
Error: Relay config permissions are too open (644) at ~/.config/mcp-vault-wrap/relay.toml
Expected 600 (owner read/write only). Fix with: chmod 600 ~/.config/mcp-vault-wrap/relay.toml
```

**Unrecognized config version:**
```
Error: Relay config version 2 is not supported by this version of mcp-vault-wrap
Update mcp-vault-wrap or check your config file.
```

**Secret not found in Keychain:**
```
Error: Secret "mcp-vault-wrap.default.GITHUB_TOKEN" not found in Keychain
Run: mcp-vault-wrap add default GITHUB_TOKEN
```

**Keychain inaccessible:**
```
Error: Cannot access macOS Keychain: session locked
Ensure your desktop session is unlocked and mcp-vault-wrap has Keychain access.
```

**Downstream server fails to start:**
```
Error: Failed to start server "github": command not found: npx
Verify the server command in ~/.config/mcp-vault-wrap/relay.toml
```

**Downstream server exits mid-session:**
```
Error: Server "github" exited with code 1
```
Relay exits with the downstream server's exit code.

### Protocol Notes

- The relay is a protocol-level MCP proxy: MCP server facade facing the host, MCP client facing the downstream server.
- All MCP messages are forwarded by default. The relay only intercepts messages for explicitly declared carve-outs.
- MVP carve-out: sampling capability is filtered from client capabilities presented downstream.
- In stdio mode, downstream server stderr passes through to the relay's stderr. This is a stdio-mode convenience, not a core relay property.

---

## 5) `add`

### Invocation

```bash
mcp-vault-wrap add <profile> <secret-name>
```

**Optional flags:**

- `--force` — overwrite an existing secret.

### Secret Input

- When stdin is a terminal: interactive prompt with no echo.
- When stdin is a pipe: read value from stdin.

No `--value` flag. Secrets must not appear in shell history.

### Success Output

**New secret:**
```
Enter secret value for GITHUB_TOKEN: ****
Stored: mcp-vault-wrap.default.GITHUB_TOKEN
```

**Overwrite with `--force`:**
```
Enter secret value for GITHUB_TOKEN: ****
Updated: mcp-vault-wrap.default.GITHUB_TOKEN
```

### Failure Cases

**Secret already exists (without `--force`):**
```
Error: Secret "mcp-vault-wrap.default.GITHUB_TOKEN" already exists in Keychain
Use --force to overwrite.
```

**Invalid secret name:**
```
Error: Invalid secret name: "MY KEY"
Names must match [a-zA-Z0-9_.-]
```

**Invalid profile name:**
```
Error: Invalid profile name: "../etc"
Names must match [a-zA-Z0-9_.-]
```

**Keychain write failure:**
```
Error: Failed to write to Keychain: access denied
Ensure mcp-vault-wrap has Keychain access in System Settings → Privacy & Security.
```

---

## 6) `remove`

### Invocation

```bash
mcp-vault-wrap remove <profile> <secret-name>
```

### Success Output

```
Removed: mcp-vault-wrap.default.GITHUB_TOKEN
```

### Failure Cases

**Secret not found:**
```
Error: Secret "mcp-vault-wrap.default.GITHUB_TOKEN" not found in Keychain
```

**Invalid name:**
```
Error: Invalid secret name: "MY KEY"
Names must match [a-zA-Z0-9_.-]
```

**Invalid profile name:**
```
Error: Invalid profile name: "../etc"
Names must match [a-zA-Z0-9_.-]
```

**Keychain access failure:**
```
Error: Failed to access Keychain: access denied
Ensure mcp-vault-wrap has Keychain access in System Settings → Privacy & Security.
```

---

## 7) `doctor`

### Invocation

```bash
mcp-vault-wrap doctor
```

No required flags. Reads the relay TOML and checks all diagnostics it can.

**Optional flags:**

- `--config <path>` — override the default relay TOML path.

### Output: All Checks Pass

```
Config: ~/.config/mcp-vault-wrap/relay.toml (valid, version 1, permissions ok)
Keychain: accessible
Server "github":
  ✓ GITHUB_TOKEN present in Keychain
Server "slack":
  ✓ SLACK_BOT_TOKEN present in Keychain
  ✓ SLACK_TEAM_ID in relay config (non-secret)
All checks passed.
```

Exit code: 0

### Output: Issues Found

```
Config: ~/.config/mcp-vault-wrap/relay.toml (valid, version 1, permissions ok)
Keychain: accessible
Server "github":
  ✗ GITHUB_TOKEN not found in Keychain
    Run: mcp-vault-wrap add default GITHUB_TOKEN
Server "slack":
  ✓ SLACK_BOT_TOKEN present in Keychain
  ✓ SLACK_TEAM_ID in relay config (non-secret)
1 issue found.
```

Exit code: non-zero

### Output: No Config

```
Config: ~/.config/mcp-vault-wrap/relay.toml not found
Run "mcp-vault-wrap migrate --host claude-desktop --servers ..." to set up.
```

Exit code: non-zero

### Output: Bad Permissions

```
Config: ~/.config/mcp-vault-wrap/relay.toml (valid, version 1)
  ✗ File permissions are too open (644). Expected 600.
    Fix with: chmod 600 ~/.config/mcp-vault-wrap/relay.toml
```

Exit code: non-zero

### Output: Keychain Inaccessible

```
Config: ~/.config/mcp-vault-wrap/relay.toml (valid, version 1)
Keychain: inaccessible (session locked)
  Ensure your desktop session is unlocked and mcp-vault-wrap has Keychain access.
Cannot check secrets — skipping server checks.
```

Exit code: non-zero

### Behavior Notes

- Doctor is diagnostic only. It never modifies config, secrets, or state.
- Doctor is only run when explicitly invoked. Other commands perform their own validations.
- Doctor does NOT attempt downstream server launchability checks in MVP.
- Exit code 0 if all checks pass, non-zero if any issues are found.
