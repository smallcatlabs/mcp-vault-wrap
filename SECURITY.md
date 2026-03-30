# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in mcp-vault-wrap, please report it responsibly. **Do not open a public GitHub issue.**

Email: security@smallcatlabs.com

Or use [GitHub's private vulnerability reporting](https://github.com/smallcatlabs/mcp-vault-wrap/security/advisories/new).

You should receive a response within 72 hours. Please include:

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

## Security Model

mcp-vault-wrap is credential-at-rest hardening for local MCP stdio workflows. It moves MCP server credentials from plaintext host configuration files into macOS Keychain and relays MCP traffic transparently.

It is **not** a complete MCP security solution.

### Protects Against

- plaintext credential scraping from host config files
- accidental sharing/committing of static credential values in host config

### Does Not Protect Against

- active same-user compromise in an unlocked session
- prompt injection or tool-output attacks
- runtime exposure characteristics of env-var injection
- full host compromise where attacker can invoke the relay

### Implementation Notes

- The relay never logs secret values at any verbosity level.
- Relay configuration (`relay.toml`) is written with mode `600` and checked at startup; the relay refuses to proceed if permissions are too open.
- Secret injection uses environment variables in MVP. Env vars have a shorter exposure window than files on disk but are not zero-exposure.
- No auto-update mechanism. Users must explicitly choose every version change.

### Claims Versioning

- Each release includes current "Protects Against" and "Does Not Protect Against" claims.
- If claims change, release notes will include a **Security Model Changes** section.
- Product and feature documentation remains claim-consistent with the current release.

## Supported Versions

Only the latest release is supported with security updates. This policy will be revisited when the project reaches stable release.
