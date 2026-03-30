# mcp-vault-wrap

Secure credential infrastructure for the MCP ecosystem.

## The Problem

Every major MCP host — Claude Desktop, Claude Code, Cursor, Windsurf, VS Code — stores API keys as plaintext strings in JSON configuration files on the user's filesystem. A typical developer's `claude_desktop_config.json` contains GitHub tokens, Slack bot tokens, database credentials, and API keys in cleartext, readable by any process running as that user.

## What mcp-vault-wrap Does

mcp-vault-wrap is a Rust CLI that sits between your MCP host and your MCP servers. It moves credentials out of plaintext config files into the macOS Keychain, resolves them at runtime, and relays MCP traffic transparently. Your MCP servers work exactly as before — they just never appear in cleartext on your filesystem again.

One command to migrate:

```bash
mcp-vault-wrap migrate --host claude-desktop --servers github,slack
```

Your secrets are moved to the Keychain. Your host config is rewritten to launch `mcp-vault-wrap run <server-name>` instead of the original server command. The relay handles everything else.

## What mcp-vault-wrap Is Not

This is credential-at-rest hardening for local development workflows. It is not a complete MCP security solution. It does not detect prompt injection, scan tool responses, pin tool definitions, or protect against a compromised host.

mcp-vault-wrap does one thing: it gets your secrets out of plaintext files and into a secure store.

## Security Model

### Protects Against

- Plaintext credential scraping from host config files
- Accidental sharing/committing of static credential values in host config

### Does Not Protect Against

- Active same-user compromise in an unlocked session
- Prompt injection or tool-output attacks
- Runtime exposure characteristics of env-var injection
- Full host compromise where attacker can invoke the relay

## Project Status

Pre-release. Implementation is in progress toward MVP. Design documentation is complete — see [docs/](docs/) for the architecture contract, product spec, design decisions, technical design spec, and implementation plan.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
