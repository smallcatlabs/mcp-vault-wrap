/// Classification of an environment variable in a registry entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EnvClassification {
    Secret,
    Config,
}

/// A compiled registry entry defining the known env vars for a server.
pub struct RegistryEntry {
    pub name: &'static str,
    pub env_vars: &'static [(&'static str, EnvClassification)],
}

/// Returns the compiled MVP registry.
pub fn registry() -> &'static [RegistryEntry] {
    &[
        RegistryEntry {
            name: "github",
            env_vars: &[("GITHUB_TOKEN", EnvClassification::Secret)],
        },
        RegistryEntry {
            name: "slack",
            env_vars: &[
                ("SLACK_BOT_TOKEN", EnvClassification::Secret),
                ("SLACK_TEAM_ID", EnvClassification::Config),
            ],
        },
    ]
}

/// Look up a registry entry by exact name match.
pub fn lookup(name: &str) -> Option<&'static RegistryEntry> {
    registry().iter().find(|entry| entry.name == name)
}

/// Returns a sorted list of all supported server names.
pub fn supported_servers() -> Vec<&'static str> {
    let mut names: Vec<_> = registry().iter().map(|e| e.name).collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_github() {
        let entry = lookup("github").unwrap();
        assert_eq!(entry.name, "github");
        assert_eq!(entry.env_vars.len(), 1);
        assert_eq!(
            entry.env_vars[0],
            ("GITHUB_TOKEN", EnvClassification::Secret)
        );
    }

    #[test]
    fn lookup_slack() {
        let entry = lookup("slack").unwrap();
        assert_eq!(entry.name, "slack");
        assert_eq!(entry.env_vars.len(), 2);
        assert_eq!(
            entry.env_vars[0],
            ("SLACK_BOT_TOKEN", EnvClassification::Secret)
        );
        assert_eq!(
            entry.env_vars[1],
            ("SLACK_TEAM_ID", EnvClassification::Config)
        );
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(lookup("unknown-server").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn supported_servers_lists_all() {
        let servers = supported_servers();
        assert_eq!(servers, vec!["github", "slack"]);
    }

    #[test]
    fn classification_equality() {
        assert_eq!(EnvClassification::Secret, EnvClassification::Secret);
        assert_ne!(EnvClassification::Secret, EnvClassification::Config);
    }
}
