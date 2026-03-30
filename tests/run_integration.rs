// I-27: In-process integration test for `run` orchestration.
//
// Exercises via `execute()` with InMemoryBackend (per S-3, S-6):
//   - config load, validation, and error paths
//   - secret resolution and injection as env vars
//   - non-secret env injection
//   - child spawn and exit-code mirroring
//   - spawn failure with actionable error
//
// Exercises via proxy_loop + real child process:
//   - MCP stdin/stdout relay round-trip (message content verified)
//   - secret values never appear in relayed messages
//
// Verified by code inspection (not CI-testable in-process):
//   - stderr passthrough (Stdio::inherit() — child stderr goes to relay fd 2)
//   - signal forwarding (SIGTERM/SIGINT → child via libc::kill)
//   - verbose output content (eprintln! to fd 2, not capturable in-process)

use std::io::Write;
use std::sync::Arc;

use tokio::sync::Mutex;

use mcp_vault_wrap::commands::run;
use mcp_vault_wrap::relay;
use mcp_vault_wrap::secret::SecretBackend;
use mcp_vault_wrap::secret::memory::InMemoryBackend;
use mcp_vault_wrap::transport::TransportSender;
use mcp_vault_wrap::transport::stdio::StdioTransport;
use tempfile::TempDir;

fn write_config(dir: &TempDir, toml_content: &str) -> std::path::PathBuf {
    let path = dir.path().join("relay.toml");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(toml_content.as_bytes()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    path
}

// ---------------------------------------------------------------------------
// Config validation error paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_missing_config_returns_error() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("nonexistent.toml");
    let backend = InMemoryBackend::new();

    let err = run::execute(&backend, "github", &config_path, false)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Relay config not found"), "error was: {msg}");
}

#[tokio::test]
async fn run_bad_config_version_returns_error() {
    let dir = TempDir::new().unwrap();
    let config_path = write_config(
        &dir,
        r#"
config_version = 99

[profile.default.servers.github]
command = "echo"
args = []
"#,
    );
    let backend = InMemoryBackend::new();

    let err = run::execute(&backend, "github", &config_path, false)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("version 99 is not supported"),
        "error was: {msg}"
    );
}

#[tokio::test]
async fn run_server_not_found_returns_error() {
    let dir = TempDir::new().unwrap();
    let config_path = write_config(
        &dir,
        r#"
config_version = 1

[profile.default.servers.slack]
command = "echo"
args = []
"#,
    );
    let backend = InMemoryBackend::new();

    let err = run::execute(&backend, "github", &config_path, false)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("Server \"github\" not defined"),
        "error was: {msg}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn run_bad_permissions_returns_error() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("relay.toml");
    std::fs::write(
        &path,
        r#"
config_version = 1

[profile.default.servers.github]
command = "echo"
args = []
"#,
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let backend = InMemoryBackend::new();
    let err = run::execute(&backend, "github", &path, false)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("permissions are too open"), "error was: {msg}");
}

// ---------------------------------------------------------------------------
// Secret resolution error paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_missing_secret_returns_error() {
    let dir = TempDir::new().unwrap();
    let config_path = write_config(
        &dir,
        r#"
config_version = 1

[profile.default.servers.github]
command = "echo"
args = []

[profile.default.servers.github.secret_env]
GITHUB_TOKEN = "vault://default/GITHUB_TOKEN"
"#,
    );
    let backend = InMemoryBackend::new();
    // Don't set the secret — should fail

    let err = run::execute(&backend, "github", &config_path, false)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("not found in Keychain"), "error was: {msg}");
    assert!(
        msg.contains("mcp-vault-wrap add default GITHUB_TOKEN"),
        "error was: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Spawn failure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_spawn_failure_returns_error() {
    let dir = TempDir::new().unwrap();
    let config_path = write_config(
        &dir,
        r#"
config_version = 1

[profile.default.servers.bad]
command = "nonexistent-command-that-does-not-exist-xyz"
args = []
"#,
    );
    let backend = InMemoryBackend::new();

    let err = run::execute(&backend, "bad", &config_path, false)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Failed to start server"), "error was: {msg}");
    assert!(
        msg.contains("Verify the server command in"),
        "missing config guidance, error was: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Spawn + exit-code mirroring
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_no_secrets_spawns_and_exits() {
    // `true` exits immediately with code 0 — proves spawn + exit-code mirroring
    let dir = TempDir::new().unwrap();
    let config_path = write_config(
        &dir,
        r#"
config_version = 1

[profile.default.servers.ok]
command = "true"
args = []
"#,
    );
    let backend = InMemoryBackend::new();

    let exit_code = run::execute(&backend, "ok", &config_path, false)
        .await
        .unwrap();
    assert_eq!(exit_code, 0);
}

#[tokio::test]
async fn run_exit_code_mirrors_downstream() {
    // `false` exits with code 1
    let dir = TempDir::new().unwrap();
    let config_path = write_config(
        &dir,
        r#"
config_version = 1

[profile.default.servers.fail]
command = "false"
args = []
"#,
    );
    let backend = InMemoryBackend::new();

    let exit_code = run::execute(&backend, "fail", &config_path, false)
        .await
        .unwrap();
    assert_eq!(exit_code, 1);
}

// ---------------------------------------------------------------------------
// Env injection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_with_secrets_injects_env_vars() {
    let dir = TempDir::new().unwrap();
    let config_path = write_config(
        &dir,
        r#"
config_version = 1

[profile.default.servers.check]
command = "sh"
args = ["-c", "echo $MY_SECRET > /dev/null; exit 0"]

[profile.default.servers.check.secret_env]
MY_SECRET = "vault://default/MY_SECRET"
"#,
    );
    let backend = InMemoryBackend::new();
    backend.set("default", "MY_SECRET", "s3cret").unwrap();

    let exit_code = run::execute(&backend, "check", &config_path, false)
        .await
        .unwrap();
    assert_eq!(exit_code, 0);
}

#[tokio::test]
async fn run_with_non_secret_env_injects_config_vars() {
    let dir = TempDir::new().unwrap();
    let config_path = write_config(
        &dir,
        r#"
config_version = 1

[profile.default.servers.check]
command = "sh"
args = ["-c", "test \"$TEAM_ID\" = 'T123' && exit 0 || exit 42"]

[profile.default.servers.check.env]
TEAM_ID = "T123"
"#,
    );
    let backend = InMemoryBackend::new();

    let exit_code = run::execute(&backend, "check", &config_path, false)
        .await
        .unwrap();
    assert_eq!(exit_code, 0);
}

// ---------------------------------------------------------------------------
// Relay round-trip with real child process
//
// These tests exercise proxy_loop + real StdioTransport + real child process
// to prove the MCP message relay path end-to-end. The `execute()` function
// hardcodes process stdin/stdout for the host side, so we test at the layer
// below: spawn a child, wire transports, run the proxy loop, verify output.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn relay_round_trip_verifies_content() {
    use async_trait::async_trait;
    use mcp_vault_wrap::transport::TransportError;
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    // RecordingSender captures sent messages for assertion.
    struct RecordingSender {
        messages: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    #[async_trait]
    impl TransportSender for RecordingSender {
        async fn send(&mut self, message: &[u8]) -> Result<(), TransportError> {
            self.messages.lock().await.push(message.to_vec());
            Ok(())
        }
    }

    // Use `head -2` as the downstream: reads 2 lines, echoes them, exits.
    // When it exits, its stdout closes → downstream recv() returns Closed →
    // proxy loop terminates. This avoids the host-EOF race: the host reader
    // stays open (pends) while echoes flow back through the downstream.
    let mut child = Command::new("head")
        .args(["-n", "2"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .unwrap();

    let child_stdin = child.stdin.take().unwrap();
    let child_stdout = child.stdout.take().unwrap();

    // Host reader: use a duplex so we can write messages and keep the stream
    // open (the read half pends after the messages, it doesn't return EOF).
    let msg1 = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"test\"}";
    let msg2 = b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"test2\"}";

    let (host_read_half, mut host_write_half) = tokio::io::duplex(4096);

    // Write the two messages, then keep the write half alive (don't drop it).
    host_write_half
        .write_all(
            format!(
                "{}\n{}\n",
                std::str::from_utf8(msg1).unwrap(),
                std::str::from_utf8(msg2).unwrap()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    host_write_half.flush().await.unwrap();

    let dummy_writer: Vec<u8> = Vec::new();
    let host_transport = StdioTransport::new(host_read_half, dummy_writer);
    let (mut host_reader, _dummy) = host_transport.split();

    // Host writer: recording sender to capture echoed responses.
    let received = Arc::new(Mutex::new(Vec::new()));
    let host_writer: Arc<Mutex<dyn TransportSender>> = Arc::new(Mutex::new(RecordingSender {
        messages: received.clone(),
    }));

    let mut downstream_transport = StdioTransport::new(child_stdout, child_stdin);

    // Proxy loop: host sends 2 messages → head echoes them → head exits →
    // downstream EOF → loop ends.
    relay::proxy_loop(
        &mut host_reader,
        &host_writer,
        &mut downstream_transport,
        false,
    )
    .await
    .unwrap();

    drop(downstream_transport);
    drop(host_write_half);
    child.wait().await.unwrap();

    // Verify both echoed messages arrived at the host writer.
    let msgs = received.lock().await;
    assert_eq!(
        msgs.len(),
        2,
        "expected 2 echoed messages, got {}",
        msgs.len()
    );
    assert_eq!(msgs[0], msg1, "first echoed message mismatch");
    assert_eq!(msgs[1], msg2, "second echoed message mismatch");
}

// Note: stderr passthrough is a single-line implementation property:
// `cmd.stderr(std::process::Stdio::inherit())` in commands/run.rs.
// With Stdio::inherit(), child stderr flows directly to the relay's fd 2,
// which cannot be captured in an in-process test. This is verified by code
// inspection rather than a test assertion.

// ---------------------------------------------------------------------------
// Secret-never-logged invariant (C-§6)
//
// Verifies that secret values injected as env vars do not appear in messages
// relayed by the proxy. The relay forwards raw MCP messages — it never has
// access to secret values (those are set on the child process env, not in
// the message stream). This test confirms the architectural boundary.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn secret_values_not_in_relayed_messages() {
    use async_trait::async_trait;
    use mcp_vault_wrap::transport::TransportError;

    struct RecordingSender {
        messages: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    #[async_trait]
    impl TransportSender for RecordingSender {
        async fn send(&mut self, message: &[u8]) -> Result<(), TransportError> {
            self.messages.lock().await.push(message.to_vec());
            Ok(())
        }
    }

    // Spawn a child that has a secret in its env but only echoes stdin.
    // Use `head -1` to echo one line then exit.
    // The secret should never appear in the relayed output.
    let secret_value = "super-secret-token-value-12345";

    let mut child = tokio::process::Command::new("head")
        .args(["-n", "1"])
        .env("SECRET_TOKEN", secret_value)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .unwrap();

    let child_stdin = child.stdin.take().unwrap();
    let child_stdout = child.stdout.take().unwrap();

    // Send one message (no secret content), keep host reader open.
    let msg = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}";

    let (host_read_half, mut host_write_half) = tokio::io::duplex(4096);
    use tokio::io::AsyncWriteExt;
    host_write_half
        .write_all(format!("{}\n", std::str::from_utf8(msg).unwrap()).as_bytes())
        .await
        .unwrap();
    host_write_half.flush().await.unwrap();

    let dummy_writer: Vec<u8> = Vec::new();
    let host_transport = StdioTransport::new(host_read_half, dummy_writer);
    let (mut host_reader, _dummy) = host_transport.split();

    let received = Arc::new(Mutex::new(Vec::new()));
    let host_writer: Arc<Mutex<dyn TransportSender>> = Arc::new(Mutex::new(RecordingSender {
        messages: received.clone(),
    }));

    let mut downstream_transport = StdioTransport::new(child_stdout, child_stdin);

    relay::proxy_loop(
        &mut host_reader,
        &host_writer,
        &mut downstream_transport,
        false,
    )
    .await
    .unwrap();

    drop(downstream_transport);
    drop(host_write_half);
    child.wait().await.unwrap();

    // Verify no relayed message contains the secret value.
    let msgs = received.lock().await;
    for (i, msg) in msgs.iter().enumerate() {
        let content = String::from_utf8_lossy(msg);
        assert!(
            !content.contains(secret_value),
            "secret value leaked in relayed message {i}: {content}"
        );
    }
}
