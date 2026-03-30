use std::collections::HashMap;
use std::path::Path;

use super::{HostConfig, HostConfigData, HostConfigError, HostServerEntry};

/// Claude Desktop host config parser.
///
/// Reads `claude_desktop_config.json`, extracting server entries from
/// the `mcpServers` object. On write, only migrated server entries are
/// modified; all other fields are preserved via the raw JSON value.
pub struct ClaudeDesktopConfig;

impl HostConfig for ClaudeDesktopConfig {
    fn parse(path: &Path) -> Result<HostConfigData, HostConfigError> {
        let content = std::fs::read_to_string(path).map_err(|e| HostConfigError::IoError {
            detail: format!("Cannot read {}: {e}", path.display()),
        })?;

        let raw: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| HostConfigError::ParseError {
                detail: e.to_string(),
            })?;

        let mcp_servers = raw
            .get("mcpServers")
            .and_then(|v| v.as_object())
            .unwrap_or(&serde_json::Map::new())
            .clone();

        let mut servers = HashMap::new();
        for (name, entry) in &mcp_servers {
            let command = entry
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let args = entry
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();

            let env = entry
                .get("env")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();

            servers.insert(name.clone(), HostServerEntry { command, args, env });
        }

        Ok(HostConfigData {
            servers,
            raw: raw.clone(),
        })
    }

    fn write(path: &Path, data: &HostConfigData) -> Result<(), HostConfigError> {
        let mut raw = data.raw.clone();

        // Ensure mcpServers exists as an object
        if raw.get("mcpServers").is_none() {
            raw.as_object_mut()
                .ok_or_else(|| HostConfigError::ParseError {
                    detail: "root is not a JSON object".to_string(),
                })?
                .insert("mcpServers".to_string(), serde_json::json!({}));
        }

        let mcp_servers = raw
            .get_mut("mcpServers")
            .and_then(|v| v.as_object_mut())
            .ok_or_else(|| HostConfigError::ParseError {
                detail: "mcpServers is not a JSON object".to_string(),
            })?;

        // Replace only the servers present in data.servers. Entries NOT in
        // data.servers are left as-is in the raw JSON, preserving any extra
        // per-server fields the host config may have (e.g. disabled, alwaysAllow).
        for (name, entry) in &data.servers {
            let mut server_obj = serde_json::Map::new();
            server_obj.insert(
                "command".to_string(),
                serde_json::Value::String(entry.command.clone()),
            );
            server_obj.insert(
                "args".to_string(),
                serde_json::Value::Array(
                    entry
                        .args
                        .iter()
                        .map(|a| serde_json::Value::String(a.clone()))
                        .collect(),
                ),
            );
            if !entry.env.is_empty() {
                let env_obj: serde_json::Map<String, serde_json::Value> = entry
                    .env
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect();
                server_obj.insert("env".to_string(), serde_json::Value::Object(env_obj));
            }
            mcp_servers.insert(name.clone(), serde_json::Value::Object(server_obj));
        }

        let output =
            serde_json::to_string_pretty(&raw).map_err(|e| HostConfigError::ParseError {
                detail: format!("Failed to serialize host config: {e}"),
            })?;

        std::fs::write(path, output.as_bytes()).map_err(|e| HostConfigError::IoError {
            detail: format!("Cannot write {}: {e}", path.display()),
        })?;

        Ok(())
    }
}

/// Resolve the host config file path for a given host name.
pub fn resolve_host_config_path(host: &str) -> Result<std::path::PathBuf, String> {
    match host {
        "claude-desktop" => {
            // ~/Library/Application Support/Claude/claude_desktop_config.json
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
            Ok(std::path::PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Claude")
                .join("claude_desktop_config.json"))
        }
        other => Err(format!(
            "Error: Unsupported host \"{other}\"\nSupported hosts: claude-desktop"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_claude_config() -> String {
        serde_json::json!({
            "mcpServers": {
                "github": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-github"],
                    "env": {
                        "GITHUB_TOKEN": "ghp_test123"
                    }
                },
                "slack": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-slack"],
                    "env": {
                        "SLACK_BOT_TOKEN": "xoxb-test",
                        "SLACK_TEAM_ID": "T01234567"
                    }
                },
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
                }
            },
            "otherSetting": true
        })
        .to_string()
    }

    #[test]
    fn parse_extracts_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, sample_claude_config()).unwrap();

        let data = ClaudeDesktopConfig::parse(&path).unwrap();
        assert_eq!(data.servers.len(), 3);

        let github = &data.servers["github"];
        assert_eq!(github.command, "npx");
        assert_eq!(
            github.args,
            vec!["-y", "@modelcontextprotocol/server-github"]
        );
        assert_eq!(github.env["GITHUB_TOKEN"], "ghp_test123");

        let slack = &data.servers["slack"];
        assert_eq!(slack.env["SLACK_BOT_TOKEN"], "xoxb-test");
        assert_eq!(slack.env["SLACK_TEAM_ID"], "T01234567");

        let fs = &data.servers["filesystem"];
        assert!(fs.env.is_empty());
    }

    #[test]
    fn write_replaces_only_specified_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, sample_claude_config()).unwrap();

        let parsed = ClaudeDesktopConfig::parse(&path).unwrap();

        // Build write data with ONLY github modified — slack and filesystem
        // are not in data.servers, so they must survive untouched in raw.
        let mut servers = HashMap::new();
        servers.insert(
            "github".to_string(),
            HostServerEntry {
                command: "mcp-vault-wrap".to_string(),
                args: vec!["run".to_string(), "github".to_string()],
                env: HashMap::new(),
            },
        );
        let data = HostConfigData {
            servers,
            raw: parsed.raw,
        };

        ClaudeDesktopConfig::write(&path, &data).unwrap();

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        // Non-server field preserved
        assert_eq!(written["otherSetting"], serde_json::json!(true));

        // github was rewritten
        assert_eq!(written["mcpServers"]["github"]["command"], "mcp-vault-wrap");
        assert_eq!(
            written["mcpServers"]["github"]["args"],
            serde_json::json!(["run", "github"])
        );
        assert!(written["mcpServers"]["github"].get("env").is_none());

        // slack and filesystem untouched — still original values from raw
        assert_eq!(written["mcpServers"]["slack"]["command"], "npx");
        assert_eq!(written["mcpServers"]["filesystem"]["command"], "npx");
    }

    #[test]
    fn write_preserves_extra_fields_on_untouched_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        // Host config with extra per-server fields (disabled, alwaysAllow)
        let config = serde_json::json!({
            "mcpServers": {
                "github": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-github"],
                    "env": { "GITHUB_TOKEN": "ghp_test" }
                },
                "slack": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-slack"],
                    "env": { "SLACK_BOT_TOKEN": "xoxb-test" },
                    "disabled": true,
                    "alwaysAllow": ["read_channel"]
                }
            }
        });
        std::fs::write(&path, config.to_string()).unwrap();

        let parsed = ClaudeDesktopConfig::parse(&path).unwrap();

        // Migrate only github — slack has extra fields that must survive
        let mut servers = HashMap::new();
        servers.insert(
            "github".to_string(),
            HostServerEntry {
                command: "mcp-vault-wrap".to_string(),
                args: vec!["run".to_string(), "github".to_string()],
                env: HashMap::new(),
            },
        );
        let data = HostConfigData {
            servers,
            raw: parsed.raw,
        };

        ClaudeDesktopConfig::write(&path, &data).unwrap();

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        // github was rewritten
        assert_eq!(written["mcpServers"]["github"]["command"], "mcp-vault-wrap");

        // slack's extra fields preserved
        assert_eq!(written["mcpServers"]["slack"]["disabled"], true);
        assert_eq!(
            written["mcpServers"]["slack"]["alwaysAllow"],
            serde_json::json!(["read_channel"])
        );
        assert_eq!(written["mcpServers"]["slack"]["command"], "npx");
    }

    #[test]
    fn parse_missing_file() {
        let result = ClaudeDesktopConfig::parse(Path::new("/nonexistent/config.json"));
        assert!(result.is_err());
    }

    #[test]
    fn parse_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "not json {{{").unwrap();

        let result = ClaudeDesktopConfig::parse(&path);
        assert!(result.is_err());
    }

    #[test]
    fn parse_empty_mcp_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"otherSetting": true}"#).unwrap();

        let data = ClaudeDesktopConfig::parse(&path).unwrap();
        assert!(data.servers.is_empty());
    }

    #[test]
    fn parse_server_without_env() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"mcpServers": {"test": {"command": "echo", "args": ["hi"]}}}"#,
        )
        .unwrap();

        let data = ClaudeDesktopConfig::parse(&path).unwrap();
        assert!(data.servers["test"].env.is_empty());
    }

    #[test]
    fn resolve_claude_desktop_path() {
        // Just verify the path ends with the expected suffix; HOME varies.
        let path = resolve_host_config_path("claude-desktop").unwrap();
        let suffix = "Library/Application Support/Claude/claude_desktop_config.json";
        assert!(
            path.to_string_lossy().ends_with(suffix),
            "expected path ending with {suffix}, got {}",
            path.display()
        );
    }

    #[test]
    fn resolve_unsupported_host() {
        let result = resolve_host_config_path("cursor");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported host"));
    }
}
