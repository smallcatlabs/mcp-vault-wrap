# mcp-vault-wrap

Rust CLI security tool that moves MCP credentials from plaintext config files into macOS Keychain and relays MCP traffic transparently.

## Key Design Principles

- **Fail-closed:** Refuse on ambiguity, never fall back silently.
- **The registry is the product:** Deterministic, curated secret classification — no heuristics.
- **Secret means credential:** Only values that grant access when leaked are classified as secrets.
- **Honest security claims:** Explicit "Protects Against" and "Does Not Protect Against" published with every release.

## Documentation

Read these docs before making changes:

- `docs/mvp-architecture-contract.md` — source of truth for MVP scope, invariants, and seams
- `docs/product-spec.md` — user-facing command behavior, output shapes, error messages
- `docs/design-decisions.md` — resolved design questions with rationale
- `docs/mvp-roadmap.md` — post-MVP progression plan

## Working Style

- Work through design decisions one at a time, back and forth, before drafting documents or writing code.
- Focus on issues that could cause architectural rework — don't be nitpicky.
- Use conventional commits: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`.
- Dual licensed: MIT OR Apache-2.0.
