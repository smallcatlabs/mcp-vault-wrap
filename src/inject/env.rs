use std::collections::HashMap;
use std::process::Command;

use super::{InjectError, Injector};

/// Sets secrets as environment variables on the child process Command.
pub struct EnvInjector;

impl Injector for EnvInjector {
    fn inject(
        &self,
        secrets: HashMap<String, String>,
        command: &mut Command,
    ) -> Result<(), InjectError> {
        command.envs(secrets);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_sets_env_vars() {
        let injector = EnvInjector;
        let mut secrets = HashMap::new();
        secrets.insert("TOKEN".to_string(), "secret-value".to_string());
        secrets.insert("API_KEY".to_string(), "key-value".to_string());

        let mut cmd = Command::new("echo");
        injector.inject(secrets, &mut cmd).unwrap();

        // Command::get_envs() returns the env vars set on the command
        let envs: HashMap<_, _> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().to_string(),
                    v.unwrap().to_string_lossy().to_string(),
                )
            })
            .collect();

        assert_eq!(envs.get("TOKEN").unwrap(), "secret-value");
        assert_eq!(envs.get("API_KEY").unwrap(), "key-value");
    }

    #[test]
    fn inject_empty_secrets_is_ok() {
        let injector = EnvInjector;
        let mut cmd = Command::new("echo");
        injector.inject(HashMap::new(), &mut cmd).unwrap();
        assert_eq!(cmd.get_envs().count(), 0);
    }

    #[test]
    fn inject_second_value_wins_on_duplicate_key() {
        let injector = EnvInjector;

        let mut secrets1 = HashMap::new();
        secrets1.insert("TOKEN".to_string(), "first".to_string());
        let mut cmd = Command::new("echo");
        injector.inject(secrets1, &mut cmd).unwrap();

        let mut secrets2 = HashMap::new();
        secrets2.insert("TOKEN".to_string(), "second".to_string());
        injector.inject(secrets2, &mut cmd).unwrap();

        // Last value wins — find the final TOKEN entry
        let last_value = cmd
            .get_envs()
            .filter(|(k, _)| *k == "TOKEN")
            .last()
            .map(|(_, v)| v.unwrap().to_string_lossy().to_string());
        assert_eq!(last_value.as_deref(), Some("second"));
    }
}
