#!/usr/bin/env bash
# I-40: Manual macOS smoke test — add → run → relay with real Keychain
#
# Verifies:
#   1. `add` writes a secret to macOS Keychain
#   2. `run` resolves the secret and spawns a downstream server
#   3. The relay proxies MCP traffic (initialize handshake)
#   4. `remove` cleans up the Keychain entry
#
# Prerequisites:
#   - macOS with unlocked desktop session
#   - cargo build --release (or cargo build)
#   - A test token value (does not need to be real for relay plumbing test)
#
# Usage:
#   ./tests/smoke/smoke-run.sh [path-to-binary]

set -euo pipefail

BINARY="${1:-target/release/mcp-vault-wrap}"
PROFILE="default"
SECRET_NAME="SMOKE_TEST_TOKEN"
SERVER_NAME="smoke-echo"
CONFIG_DIR=$(mktemp -d)
CONFIG_FILE="${CONFIG_DIR}/relay.toml"
TEST_TOKEN="smoke-test-value-$(date +%s)"
PASS=0
FAIL=0
CLEANUP_ITEMS=()

# --- Helpers ---

pass() { echo "  ✓ $1"; PASS=$((PASS + 1)); }
fail() { echo "  ✗ $1"; FAIL=$((FAIL + 1)); }
cleanup() {
    echo ""
    echo "=== Cleanup ==="

    # Remove Keychain entry (ignore errors if already removed)
    "$BINARY" remove "$PROFILE" "$SECRET_NAME" 2>/dev/null && echo "  Removed Keychain entry" || echo "  Keychain entry already removed"

    # Remove temp config
    rm -rf "$CONFIG_DIR" && echo "  Removed temp config dir: $CONFIG_DIR"

    echo ""
    if [ "$FAIL" -eq 0 ]; then
        echo "=== ALL $PASS CHECKS PASSED ==="
    else
        echo "=== $FAIL FAILED, $PASS PASSED ==="
        exit 1
    fi
}
trap cleanup EXIT

# --- Checks ---

echo "=== I-40: Smoke Test — add → run → relay ==="
echo "Binary: $BINARY"
echo "Config: $CONFIG_FILE"
echo ""

# Check binary exists
if [ ! -x "$BINARY" ]; then
    echo "Error: Binary not found at $BINARY"
    echo "Run: cargo build --release"
    exit 1
fi

# Step 1: Add secret to Keychain
echo "--- Step 1: add secret to Keychain ---"
echo "$TEST_TOKEN" | "$BINARY" add "$PROFILE" "$SECRET_NAME" --force
if [ $? -eq 0 ]; then
    pass "add wrote secret to Keychain"
else
    fail "add failed to write secret"
fi

# Step 2: Write a minimal relay TOML for the smoke-echo server
# We use `cat` as the downstream "server" — it echoes stdin to stdout,
# which is enough to verify the relay plumbing.
echo "--- Step 2: create test relay config ---"
cat > "$CONFIG_FILE" <<EOF
config_version = 1

[profile.default.servers.${SERVER_NAME}]
command = "cat"
args = []
secret_env.${SECRET_NAME} = "vault://default/${SECRET_NAME}"
EOF
chmod 600 "$CONFIG_FILE"
pass "wrote relay config with permissions 600"

# Step 3: Run doctor to verify config + secret
echo "--- Step 3: doctor check ---"
if "$BINARY" doctor --config "$CONFIG_FILE" 2>&1; then
    pass "doctor reports all checks passed"
else
    fail "doctor found issues"
fi

# Step 4: Test relay with an MCP initialize handshake
echo "--- Step 4: relay MCP round-trip ---"

# The initialize request per MCP spec
INIT_REQUEST='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{"sampling":{}},"clientInfo":{"name":"smoke-test","version":"0.1.0"}}}'

# Since we're using `cat` as the downstream, it will echo back whatever
# the relay sends it. This verifies:
#   - Secret was resolved from Keychain (relay started successfully)
#   - Transport plumbing works (message reached downstream and came back)
#   - Sampling capability was stripped (we can check the forwarded message)

RESPONSE=$(echo "$INIT_REQUEST" | timeout 5 "$BINARY" run "$SERVER_NAME" --config "$CONFIG_FILE" 2>/dev/null || true)

if [ -n "$RESPONSE" ]; then
    pass "relay produced output (transport plumbing works)"

    # Verify sampling was stripped from the forwarded message
    if echo "$RESPONSE" | grep -q '"sampling"'; then
        fail "sampling capability was NOT stripped from initialize request"
    else
        pass "sampling capability stripped from forwarded initialize request"
    fi
else
    fail "relay produced no output (transport or secret resolution failed)"
fi

# Step 5: Verify remove works
echo "--- Step 5: remove secret ---"
if "$BINARY" remove "$PROFILE" "$SECRET_NAME" 2>&1; then
    pass "remove deleted Keychain entry"
else
    fail "remove failed"
fi

# Step 6: Verify run fails after secret removal
echo "--- Step 6: verify fail-closed after secret removal ---"
if echo '{}' | timeout 5 "$BINARY" run "$SERVER_NAME" --config "$CONFIG_FILE" 2>&1; then
    fail "run should have failed with missing secret"
else
    pass "run correctly failed with missing Keychain entry"
fi
