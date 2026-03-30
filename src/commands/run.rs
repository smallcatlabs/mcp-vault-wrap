use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::process::Command;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::Mutex;

use crate::config::{self, ConfigError, RelayConfig, VaultUri};
use crate::relay;
use crate::secret::{SecretBackend, SecretError};
use crate::transport::TransportSender;
use crate::transport::stdio::StdioTransport;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum RunError {
    ConfigNotFound {
        path: String,
    },
    ConfigError(ConfigError),
    ServerNotFound {
        server: String,
        path: String,
    },
    SecretResolution {
        detail: String,
    },
    SpawnFailure {
        server: String,
        detail: String,
        config_path: String,
    },
    DownstreamExit {
        server: String,
        code: i32,
    },
    RelayError {
        detail: String,
    },
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::ConfigNotFound { path } => {
                write!(
                    f,
                    "Error: Relay config not found at {path}\n\
                     Run \"mcp-vault-wrap migrate --host claude-desktop --servers ...\" to set up."
                )
            }
            RunError::ConfigError(e) => write!(f, "Error: {e}"),
            RunError::ServerNotFound { server, path } => {
                write!(
                    f,
                    "Error: Server \"{server}\" not defined in {path}\n\
                     Run \"mcp-vault-wrap doctor\" to diagnose configuration issues."
                )
            }
            RunError::SecretResolution { detail } => {
                write!(f, "Error: {detail}")
            }
            RunError::SpawnFailure {
                server,
                detail,
                config_path,
            } => {
                write!(
                    f,
                    "Error: Failed to start server \"{server}\": {detail}\n\
                     Verify the server command in {config_path}"
                )
            }
            RunError::DownstreamExit { server, code } => {
                write!(f, "Error: Server \"{server}\" exited with code {code}")
            }
            RunError::RelayError { detail } => {
                write!(f, "Error: Relay failure: {detail}")
            }
        }
    }
}

impl std::error::Error for RunError {}

// ---------------------------------------------------------------------------
// Default config path
// ---------------------------------------------------------------------------

pub fn default_config_path() -> PathBuf {
    dirs_path()
}

fn dirs_path() -> PathBuf {
    let mut path = home_dir();
    path.push(".config");
    path.push("mcp-vault-wrap");
    path.push("relay.toml");
    path
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}

// ---------------------------------------------------------------------------
// Core orchestration
// ---------------------------------------------------------------------------

/// Load and validate relay config from the given path.
fn load_config(config_path: &Path) -> Result<RelayConfig, RunError> {
    if !config_path.exists() {
        return Err(RunError::ConfigNotFound {
            path: config_path.display().to_string(),
        });
    }

    // Check permissions before reading content
    config::validate::check_permissions(config_path).map_err(RunError::ConfigError)?;

    let content = std::fs::read_to_string(config_path).map_err(|e| {
        RunError::ConfigError(ConfigError::IoError {
            detail: format!("Cannot read {}: {e}", config_path.display()),
        })
    })?;

    let relay_config = config::deserialize(&content).map_err(RunError::ConfigError)?;

    // Validate version and vault URIs
    config::validate::validate(&relay_config).map_err(RunError::ConfigError)?;

    Ok(relay_config)
}

/// Resolve all vault:// URIs for a server's secret_env, returning env var name → secret value.
fn resolve_secrets(
    secret_env: &HashMap<String, String>,
    backend: &dyn SecretBackend,
) -> Result<HashMap<String, String>, RunError> {
    let mut resolved = HashMap::new();
    for (env_name, uri_str) in secret_env {
        let uri = VaultUri::parse(uri_str).map_err(RunError::ConfigError)?;
        let value = backend
            .get(&uri.profile, &uri.secret_name)
            .map_err(|e| match e {
                SecretError::NotFound { service } => RunError::SecretResolution {
                    detail: format!(
                        "Secret \"{service}\" not found in Keychain\n\
                         Run: mcp-vault-wrap add {} {}",
                        uri.profile, uri.secret_name
                    ),
                },
                SecretError::AccessDenied { detail } => RunError::SecretResolution {
                    detail: format!(
                        "Cannot access macOS Keychain: {detail}\n\
                         Ensure your desktop session is unlocked and mcp-vault-wrap has Keychain access."
                    ),
                },
                SecretError::BackendError { detail } => RunError::SecretResolution {
                    detail: format!("Secret backend error: {detail}"),
                },
            })?;
        resolved.insert(env_name.clone(), value);
    }
    Ok(resolved)
}

/// Run the relay for the given server. This is the main entry point for `mcp-vault-wrap run`.
pub async fn execute(
    backend: &dyn SecretBackend,
    server_name: &str,
    config_path: &Path,
    verbose: bool,
) -> Result<i32, RunError> {
    // 1-2. Load and validate config
    let relay_config = load_config(config_path)?;

    if verbose {
        eprintln!("[mcp-vault-wrap] Warning: verbose mode may log sensitive metadata");
        eprintln!(
            "[mcp-vault-wrap] Config: {} (version {})",
            config_path.display(),
            relay_config.config_version
        );
    }

    // 3. Look up server in profile.default
    let default_profile =
        relay_config
            .profile
            .get("default")
            .ok_or_else(|| RunError::ServerNotFound {
                server: server_name.to_string(),
                path: config_path.display().to_string(),
            })?;

    let server_config =
        default_profile
            .servers
            .get(server_name)
            .ok_or_else(|| RunError::ServerNotFound {
                server: server_name.to_string(),
                path: config_path.display().to_string(),
            })?;

    // 4-5. Authenticate and resolve secrets
    backend.authenticate().map_err(|e| match e {
        SecretError::AccessDenied { detail } => RunError::SecretResolution {
            detail: format!(
                "Cannot access macOS Keychain: {detail}\n\
                 Ensure your desktop session is unlocked and mcp-vault-wrap has Keychain access."
            ),
        },
        other => RunError::SecretResolution {
            detail: other.to_string(),
        },
    })?;

    let secrets = resolve_secrets(&server_config.secret_env, backend)?;

    if verbose {
        let secret_count = secrets.len();
        eprintln!(
            "[mcp-vault-wrap] Resolved {secret_count} secret{} for server \"{server_name}\"",
            if secret_count == 1 { "" } else { "s" }
        );
    }

    // 6. Build command
    let mut cmd = Command::new(&server_config.command);
    cmd.args(&server_config.args);

    // 7. Inject secrets as env vars (EnvInjector pattern, applied directly
    // to tokio::process::Command since the Injector trait uses std::process::Command)
    cmd.envs(secrets);

    // 8. Set non-secret env vars
    cmd.envs(&server_config.env);

    // 9. Configure stdio
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::inherit());

    if verbose {
        let full_cmd = format!("{} {}", server_config.command, server_config.args.join(" "));
        eprintln!("[mcp-vault-wrap] Starting: {full_cmd}");
    }

    // 10. Spawn
    let mut child = cmd.spawn().map_err(|e| RunError::SpawnFailure {
        server: server_name.to_string(),
        detail: e.to_string(),
        config_path: config_path.display().to_string(),
    })?;

    let child_stdin = child.stdin.take().unwrap();
    let child_stdout = child.stdout.take().unwrap();

    // 11. Wire transports
    //
    // The host-facing transport is split into independent reader and writer
    // halves. The writer is wrapped in Arc<Mutex<>> as the D-10 sampling
    // expansion seam: post-MVP, a sampling handler can clone this handle to
    // send messages back to the host without restructuring the proxy loop.
    // In MVP, only the proxy loop borrows the writer through the Arc.
    let host_stdin = tokio::io::stdin();
    let host_stdout = tokio::io::stdout();
    let host_transport = StdioTransport::new(host_stdin, host_stdout);
    let (mut host_reader, host_writer) = host_transport.split();
    let host_send_handle: Arc<Mutex<dyn TransportSender>> = Arc::new(Mutex::new(host_writer));

    let mut downstream_transport = StdioTransport::new(child_stdout, child_stdin);

    // 12. Set up signal forwarding
    let child_id = child.id();

    // 13. Enter proxy loop with signal handling
    //
    // The proxy loop naturally terminates when either side hits EOF. When the
    // child exits, its stdout closes → downstream recv() returns Closed → the
    // proxy loop returns. This guarantees all final child output is drained
    // before we proceed to wait(). We must NOT race child.wait() against the
    // proxy loop, as that could discard the child's final stdout messages.
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to register SIGTERM");
    let mut sigint = signal(SignalKind::interrupt()).expect("failed to register SIGINT");

    let forward_signal: Option<libc::c_int> = tokio::select! {
        result = relay::proxy_loop(&mut host_reader, &host_send_handle, &mut downstream_transport, verbose) => {
            if let Err(e) = result
                && verbose
            {
                eprintln!("[mcp-vault-wrap] Relay error: {e}");
            }
            None
        }
        _ = sigterm.recv() => Some(libc::SIGTERM),
        _ = sigint.recv() => Some(libc::SIGINT),
    };

    if let Some(sig) = forward_signal {
        // Forward the same signal to the child for clean shutdown
        if let Some(pid) = child_id {
            unsafe {
                libc::kill(pid as i32, sig);
            }
        }
    }

    // Close child stdin so it sees EOF if it's still running
    drop(downstream_transport);

    // Always wait for child exit after proxy loop or signal
    let exit_code = match child.wait().await {
        Ok(status) => status.code().unwrap_or(1),
        Err(_) => 1,
    };

    Ok(exit_code)
}

/// Entry point from main — resolves the config path default and calls execute.
pub async fn run(
    backend: &dyn SecretBackend,
    server_name: &str,
    config_override: Option<&Path>,
    verbose: bool,
) -> Result<i32, RunError> {
    let config_path = config_override
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);

    execute(backend, server_name, &config_path, verbose).await
}
