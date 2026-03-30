#!/usr/bin/env bash
# I-41: Manual macOS smoke test — migrate end-to-end on Claude Desktop config
#
# Verifies:
#   1. `migrate --dry-run` previews changes without writing
#   2. `migrate` writes secrets to Keychain, generates relay TOML, rewrites host config
#   3. The rewritten host config references `mcp-vault-wrap run <server>`
#   4. `doctor` passes after migration
#   5. Cleanup restores original state
#
# How it works:
#   The migrate command resolves the host config path from $HOME via --host.
#   This script creates a fake HOME with a synthetic Claude Desktop config,
#   so it works without Claude Desktop installed and never touches your
#   real config.
#
# Prerequisites:
#   - macOS with unlocked desktop session
#   - cargo build --release (or cargo build)
#
# Usage:
#   ./tests/smoke/smoke-migrate.sh [path-to-binary]

set -euo pipefail

BINARY="$(cd "$(dirname "${1:-target/release/mcp-vault-wrap}")" && pwd)/$(basename "${1:-target/release/mcp-vault-wrap}")"
WORK_DIR=$(mktemp -d)

# Set up fake HOME with synthetic Claude Desktop config
FAKE_HOME="$WORK_DIR/fakehome"
FAKE_CLAUDE_DIR="$FAKE_HOME/Library/Application Support/Claude"
FAKE_CONFIG="$FAKE_CLAUDE_DIR/claude_desktop_config.json"
FAKE_RELAY_DIR="$FAKE_HOME/.config/mcp-vault-wrap"
FAKE_RELAY="$FAKE_RELAY_DIR/relay.toml"

mkdir -p "$FAKE_CLAUDE_DIR"

# Symlink the real Keychains directory so macOS Security framework
# can find the login keychain when HOME is overridden
ln -s "$HOME/Library/Keychains" "$FAKE_HOME/Library/Keychains"

PASS=0
FAIL=0

# --- Helpers ---

pass() { echo "  ✓ $1"; PASS=$((PASS + 1)); }
fail() { echo "  ✗ $1"; FAIL=$((FAIL + 1)); }

# Run the binary with HOME overridden to the fake home
run_mvw() {
    HOME="$FAKE_HOME" "$BINARY" "$@"
}

cleanup() {
    echo ""
    echo "=== Cleanup ==="

    # Remove any secrets we may have written
    for secret in GITHUB_TOKEN SLACK_BOT_TOKEN; do
        "$BINARY" remove default "$secret" 2>/dev/null && echo "  Removed $secret from Keychain" || true
    done

    rm -rf "$WORK_DIR" && echo "  Removed temp dir: $WORK_DIR"

    echo ""
    if [ "$FAIL" -eq 0 ]; then
        echo "=== ALL $PASS CHECKS PASSED ==="
    else
        echo "=== $FAIL FAILED, $PASS PASSED ==="
        exit 1
    fi
}
trap cleanup EXIT

# --- Pre-flight ---

echo "=== I-41: Smoke Test — migrate end-to-end ==="
echo "Binary: $BINARY"
echo "Work dir: $WORK_DIR"
echo "Fake HOME: $FAKE_HOME"
echo ""

if [ ! -x "$BINARY" ]; then
    echo "Error: Binary not found at $BINARY"
    echo "Run: cargo build --release"
    exit 1
fi

# Write synthetic Claude Desktop config with both registry servers
cat > "$FAKE_CONFIG" <<'TESTJSON'
{
  "mcpServers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": {
        "GITHUB_TOKEN": "ghp_test1234567890"
      }
    },
    "slack": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-slack"],
      "env": {
        "SLACK_BOT_TOKEN": "xoxb-test-token-1234",
        "SLACK_TEAM_ID": "T01234567"
      }
    }
  }
}
TESTJSON
echo "  Created synthetic config with github + slack servers"
echo ""

SERVERS_TO_MIGRATE="github,slack"
echo "Servers to migrate: $SERVERS_TO_MIGRATE"
echo ""

# Step 1: Dry run
echo "--- Step 1: migrate --dry-run ---"
DRY_OUTPUT=$(run_mvw migrate --host claude-desktop --servers "$SERVERS_TO_MIGRATE" --dry-run 2>&1) || true
echo "$DRY_OUTPUT"
echo ""

if echo "$DRY_OUTPUT" | grep -q "\[dry-run\]"; then
    pass "dry-run produced prefixed output"
else
    fail "dry-run output missing [dry-run] prefix"
fi

if echo "$DRY_OUTPUT" | grep -q "No changes made"; then
    pass "dry-run reports no changes made"
else
    fail "dry-run missing 'No changes made' confirmation"
fi

# Verify nothing was written
if [ -f "$FAKE_RELAY" ]; then
    fail "dry-run wrote relay TOML (should not have)"
else
    pass "dry-run did not write relay TOML"
fi

if [ -f "${FAKE_CONFIG}.bak" ]; then
    fail "dry-run created backup (should not have)"
else
    pass "dry-run did not create backup"
fi

# Step 2: Real migration
echo ""
echo "--- Step 2: migrate (real) ---"
MIGRATE_OUTPUT=$(run_mvw migrate --host claude-desktop --servers "$SERVERS_TO_MIGRATE" 2>&1)
echo "$MIGRATE_OUTPUT"
echo ""

if [ -f "$FAKE_RELAY" ]; then
    pass "relay TOML created"
else
    fail "relay TOML not created"
fi

if [ -f "${FAKE_CONFIG}.bak" ]; then
    pass "backup created"
else
    fail "backup not created"
fi

# Step 3: Verify relay TOML content
echo "--- Step 3: verify relay TOML ---"
if [ -f "$FAKE_RELAY" ]; then
    echo "  Relay TOML contents:"
    cat "$FAKE_RELAY" | sed 's/^/    /'
    echo ""

    if grep -q "config_version = 1" "$FAKE_RELAY"; then
        pass "relay TOML has config_version = 1"
    else
        fail "relay TOML missing config_version"
    fi

    if grep -q "vault://" "$FAKE_RELAY"; then
        pass "relay TOML contains vault:// references"
    else
        fail "relay TOML missing vault:// references"
    fi

    # Check permissions (macOS stat format)
    PERMS=$(stat -f "%Lp" "$FAKE_RELAY" 2>/dev/null || stat -c "%a" "$FAKE_RELAY" 2>/dev/null)
    if [ "$PERMS" = "600" ]; then
        pass "relay TOML permissions are 600"
    else
        fail "relay TOML permissions are $PERMS (expected 600)"
    fi
fi

# Step 4: Verify host config was rewritten
echo ""
echo "--- Step 4: verify rewritten host config ---"
if grep -q "mcp-vault-wrap" "$FAKE_CONFIG"; then
    pass "host config references mcp-vault-wrap"
else
    fail "host config does not reference mcp-vault-wrap"
fi

# Secrets should no longer be in host config
if grep -q "ghp_\|xoxb-" "$FAKE_CONFIG" 2>/dev/null; then
    fail "host config still contains secret values"
else
    pass "host config no longer contains secret values"
fi

echo ""
echo "  Rewritten host config:"
cat "$FAKE_CONFIG" | sed 's/^/    /'

# Step 5: Doctor check
echo ""
echo "--- Step 5: doctor ---"
if run_mvw doctor 2>&1; then
    pass "doctor reports all checks passed after migration"
else
    fail "doctor found issues after migration"
fi

# Step 6: Verify migrate refuses to overwrite existing TOML
echo ""
echo "--- Step 6: verify TOML overwrite refusal ---"
# Restore the original config so we can attempt re-migration
cp "${FAKE_CONFIG}.bak" "$FAKE_CONFIG"
rm -f "${FAKE_CONFIG}.bak"
RERUN_OUTPUT=$(run_mvw migrate --host claude-desktop --servers "$SERVERS_TO_MIGRATE" 2>&1 || true)
if echo "$RERUN_OUTPUT" | grep -qi "already exists"; then
    pass "migrate correctly refuses when relay TOML already exists"
else
    fail "migrate did not refuse existing relay TOML"
fi
