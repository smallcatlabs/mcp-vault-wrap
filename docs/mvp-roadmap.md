# mcp-vault-wrap Post-MVP Roadmap

_March 2026_

---

## Overview

This document describes the planned progression from the MVP (local macOS CLI) through two subsequent milestones: Docker local mode (MVP+1) and Docker remote single-user mode (MVP+2). Each milestone changes exactly one major dimension of the system, allowing incremental validation without solving multiple hard problems simultaneously.

This document is informational and forward-looking. It does not modify the MVP Architecture Contract. Implementation details will be specified in their own architecture and design documents when each milestone is scoped for delivery.

---

## Milestone Progression

| Milestone | Trust model | Transport | Secret source | Secret delivery | Packaging |
|-----------|------------|-----------|---------------|-----------------|-----------|
| **MVP** | Single user, local | stdio | macOS Keychain | Direct API call | Native macOS binary |
| **MVP+1** | Single user, local | Unix domain socket | macOS Keychain (host-side) | Stdin pipe into container | Docker container |
| **MVP+2** | Single user, remote | HTTP/TLS | In-memory (web UI) | User enters via web frontend | Docker container |

Each milestone preserves the relay core (`ProxyHandler`, `SecretBackend` trait, `vault://` URI resolution, two-phase server lifecycle) unchanged. What changes is the transport layer, the secret delivery path, and the packaging.

---

## MVP+1: Docker Local Mode

### What changes from MVP

The relay runs inside a Docker container on the developer's local machine instead of as a bare process. The host's MCP client communicates with the container via a bind-mounted Unix domain socket rather than stdio. Secrets are read from the macOS Keychain on the host and piped into the container at startup.

### Why this milestone matters

It validates three things the MVP's architectural seams promise but don't prove: that the transport abstraction works (Unix socket instead of stdio), that the `InMemoryBackend` works as a production backend (not just for tests), and that the container build pipeline produces a working image. It does all this without changing the trust model — still one user, still local, still the host's Keychain as the source of truth.

### Key components

**Transport:** The `ProxyHandler` serves over a Unix domain socket instead of stdio. rmcp's `ServiceExt::serve()` accepts a `UnixStream` directly. The host MCP client configuration points at the socket path rather than a command.

**Secret delivery:** A new `export` subcommand on the host-side binary reads secrets from the Keychain for a given profile and serializes them as JSON to stdout. This is piped into the container's stdin at launch. The container-side relay reads stdin once, populates `InMemoryBackend`, closes stdin, and begins serving on the Unix socket.

```bash
mcp-vault-wrap export --profile default | docker run -i --rm \
  --read-only --network=none \
  -v /var/run/mcp-vault-wrap.sock:/var/run/mcp-vault-wrap.sock \
  mcp-vault-wrap:latest --transport unix --secrets-from stdin
```

**Container hardening:** Read-only filesystem, no network access (the container communicates only via the bind-mounted socket), non-root user, minimal Linux capabilities. The binary is statically linked against musl (cross-compilation already required by the MVP contract).

**`InMemoryBackend` promotion:** The same `InMemoryBackend` used for tests in MVP becomes the production backend inside the container. Secrets are loaded from stdin at startup and held in process memory for the relay's lifetime.

### What does NOT change

The `ProxyHandler` relay logic, the `SecretBackend` trait, the `vault://` URI resolution, the TOML config format, the server registry, the name validation, and the fail-closed error handling are all unchanged. The host-side `migrate` and `doctor` commands continue to work as before — migration still targets the host config and the host Keychain. The container is purely a runtime packaging choice for the relay.

### Estimated effort

2–3 weeks. Most architectural seams exist. The new work is the `export` subcommand, the Unix socket transport wiring, the Dockerfile, and CI pipeline changes to build and sign the container image.

---

## MVP+2: Docker Remote Single-User Mode

### What changes from MVP+1

The relay container runs on a remote host (a VPS, a cloud instance, a team server) rather than the developer's local machine. The MCP host connects over HTTP/TLS instead of a Unix socket. Secrets are managed through a minimal web frontend rather than piped from a local Keychain.

### Why this milestone matters

This is the step where the developer never sees API keys. Secrets are entered once through the web UI on the remote host, held in the relay's memory, and injected into downstream MCP servers at runtime. The developer's local machine has no credentials — only a connection URL to the relay. This is the minimum viable demonstration of a credential-zero local development workflow.

This milestone intentionally does not aim for production scale. It demonstrates the concept for a single user with a single relay instance. Multi-user, high availability, and cloud secret backend integration are future work.

### Key components

**Transport:** HTTP with TLS, served via an embedded web server (e.g., axum). rmcp's `StreamableHttpService` provides the MCP-over-HTTP layer. TLS is non-negotiable — secrets transit the network, and the admin UI accepts credentials. For MVP+2, a self-signed certificate with documented generation instructions is acceptable. Let's Encrypt or automated cert provisioning is future work.

**Secret management via web frontend:** A minimal web UI replaces the host-side Keychain as the secret source. The UI provides:

- Login with a single pre-configured password (set via environment variable or config file on the server at deployment time; no user registration, no password reset, no email)
- List configured secrets (names only, never values)
- Add a secret (name + value)
- Remove a secret
- Relay status (configured servers, connectivity state)

The frontend is static HTML with a small server-side API. Not a single-page application, not a framework-heavy build. The goal is a frontend that a security-conscious user can audit by reading the source in 15 minutes.

**Secret storage:** Secrets entered through the web UI are stored in `InMemoryBackend`. They live only in the relay process's memory. If the relay restarts, secrets must be re-entered through the UI. This is a deliberate MVP+2 tradeoff: it avoids building encrypted file persistence or cloud secret backend integration while remaining honest about the limitation. Persistent storage is future work.

**Frontend security posture:** Simple but correct — not simple and sloppy. The web UI for a security tool must not itself be a vulnerability. MVP+2 requirements:

- HTTPS only (no plaintext HTTP, even for redirects)
- Session management with secure, httponly, samesite cookies
- CSRF protection on all state-changing endpoints
- No secret values ever rendered in HTML responses or API responses
- Content-Security-Policy headers to mitigate XSS
- Rate limiting on the login endpoint

These are standard practices, not novel engineering. But they must be done correctly because a security tool with a vulnerable admin panel destroys credibility.

**Authentication between MCP host and relay:** The MCP host connects to the relay as a remote MCP server. For MVP+2 single-user mode, a shared secret (bearer token or API key, configured on both sides) is sufficient. The relay validates the token on each connection. mTLS or OAuth integration is future work.

### What does NOT change

The `ProxyHandler` relay logic, the `SecretBackend` trait, the `vault://` URI resolution, the TOML config format (with `[transport]` section now populated), the server registry, the name validation, and the fail-closed error handling are all unchanged. The `InMemoryBackend` is the same code serving its third use case (test in MVP, stdin-fed in MVP+1, web-UI-fed in MVP+2).

### What is explicitly deferred beyond MVP+2

- Multi-user support (multiple authenticated identities with separate secret sets)
- Cloud secret backends (AWS Secrets Manager, HashiCorp Vault, etc.)
- Persistent secret storage (encrypted file or database)
- Automated TLS certificate provisioning
- High availability or clustering
- Deployment automation or managed hosting

### Estimated effort

4–6 weeks. The transport switch (HTTP/TLS) and `StreamableHttpService` integration are moderate. The web frontend is small in scope but demands care around security correctness. The largest design question is TLS certificate management — self-signed with documentation is the pragmatic MVP+2 answer.

---

## Architectural Seam Validation

Each milestone validates seams that the MVP contract requires but doesn't exercise:

| Seam | MVP (specified) | MVP+1 (validated) | MVP+2 (validated) |
|------|----------------|-------------------|-------------------|
| Transport abstraction (`ProxyHandler` decoupled from stdio) | Required | Unix socket | HTTP/TLS |
| `InMemoryBackend` | Tests only | Production (stdin-fed) | Production (web-UI-fed) |
| `SecretBackend::authenticate()` | No-op | No-op | No-op (future: cloud backend auth) |
| `Option<TransportConfig>` in TOML | Absent/defaults to stdio | `type = "unix"` | `type = "http"` |
| Two-phase lifecycle (resolve → launch) | Both local | Resolve on host, launch in container | Both in container |
| Cross-compilation (musl Linux target) | CI builds, not shipped | Shipped in container | Shipped in container |
| Host-side service handle (`Arc`) | Stored, unused | Stored, unused | Stored, unused (sampling still deferred) |

---

## `InMemoryBackend` Progression

The `InMemoryBackend` is a single implementation that serves three distinct roles across the milestones:

| Milestone | Role | How secrets are loaded | Lifetime |
|-----------|------|----------------------|----------|
| MVP | Test backend | Programmatically in test setup | Test duration |
| MVP+1 | Container production backend | Stdin JSON pipe at container startup | Container process lifetime |
| MVP+2 | Remote production backend | Web UI API calls during operation | Relay process lifetime (lost on restart) |

This progression validates the `SecretBackend` abstraction: three use cases, same interface, same implementation, zero code changes to the relay core.

---

## Security Model Evolution

The MVP architecture contract (Section 6) defines explicit "Protects Against" and "Does Not Protect Against" claims with a versioning rule. Each milestone extends the protection boundary:

**MVP** protects against plaintext credential scraping from host config files on the local filesystem.

**MVP+1** adds: credentials never exist on the container's filesystem (in-memory only, read-only container filesystem, no network access from container). The host Keychain remains the source of truth. The attack surface for credential extraction narrows from "read a JSON file" to "dump process memory of a running container."

**MVP+2** adds: credentials never exist on the developer's local machine at all. The developer's host config contains only a connection URL to the remote relay. Credential extraction requires access to the remote host's relay process memory or the web UI session. The security boundary shifts from the developer's laptop to the relay server.

Each milestone's release documentation must update the claims per the contract's versioning rules (Section 6).

---

## Open Questions for Future Scoping

These do not need answers now. They are recorded to inform future architecture decisions.

- **Persistence for MVP+2:** In-memory-only means restart = re-enter all secrets. Is this acceptable for the target use case, or does MVP+2 need encrypted-at-rest storage? The answer depends on how the tool is actually used — if the relay runs on a long-lived VPS that rarely restarts, in-memory may be fine. If it's on ephemeral infrastructure, it's a dealbreaker.
- **Multi-user path from MVP+2:** Single-user remote mode uses a shared password and a flat secret namespace. Multi-user requires identity → secret set mapping, which means either namespacing secrets per user or introducing an authorization layer. The `SecretBackend` trait would need a caller identity parameter on `get_secret`. This is the largest architectural change on the horizon and should be designed carefully rather than grafted onto the single-user model.
- **Cloud backend priority:** AWS Secrets Manager, HashiCorp Vault, and 1Password CLI are the most requested integrations. Each is a `SecretBackend` implementation. The question is whether to add one alongside MVP+2 (replacing `InMemoryBackend` with a durable backend) or keep MVP+2 simple and add cloud backends as a separate milestone.
- **Managed hosting:** If the tool gains traction, a natural next step is a hosted version where Anthropic or the community runs relay infrastructure. This is a product question, not a technical one, but it would change the security model significantly (third-party holds credentials).