# macOS Smoke Tests

Manual smoke tests for verifying mcp-vault-wrap against real macOS Keychain. These must be run on a macOS machine with a desktop session (Keychain requires an unlocked session). Claude Desktop is **not** required — the migrate test uses a synthetic config.

## Prerequisites

- macOS with Keychain access
- Rust toolchain installed (see `rust-toolchain.toml`)

## Test Scripts

| Script | Covers | Implementation Plan |
|--------|--------|-------------------|
| `smoke-run.sh` | add → run → relay round-trip with real Keychain | I-40 |
| `smoke-migrate.sh` | migrate end-to-end with synthetic Claude Desktop config | I-41 |

## Running

```bash
# Build first
cargo build --release

# I-40: Keychain + relay smoke test
./tests/smoke/smoke-run.sh

# I-41: Migration smoke test (uses synthetic config, no Claude Desktop needed)
./tests/smoke/smoke-migrate.sh
```

Both scripts clean up after themselves (Keychain entries and temp directories).
