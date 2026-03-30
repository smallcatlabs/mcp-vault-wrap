# macOS Smoke Tests

Manual smoke tests for verifying mcp-vault-wrap against real macOS Keychain and Claude Desktop. These must be run on a macOS machine with a desktop session (Keychain requires an unlocked session).

## Prerequisites

- macOS with Keychain access
- Rust toolchain installed (see `rust-toolchain.toml`)
- Claude Desktop installed (for I-41 migrate test)
- A GitHub personal access token (for the relay test against a real MCP server)

## Test Scripts

| Script | Covers | Implementation Plan |
|--------|--------|-------------------|
| `smoke-run.sh` | add → run → relay round-trip with real Keychain | I-40 |
| `smoke-migrate.sh` | migrate end-to-end on Claude Desktop config | I-41 |

## Running

```bash
# Build first
cargo build --release

# I-40: Keychain + relay smoke test
./tests/smoke/smoke-run.sh

# I-41: Migration smoke test (requires Claude Desktop config)
./tests/smoke/smoke-migrate.sh
```

Each script is interactive — it will prompt for confirmation at destructive steps and clean up after itself.
