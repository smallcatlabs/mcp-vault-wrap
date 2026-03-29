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
