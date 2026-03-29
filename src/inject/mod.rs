pub mod env;

use std::collections::HashMap;
use std::fmt;
use std::process::Command;

pub trait Injector {
    fn inject(
        &self,
        secrets: HashMap<String, String>,
        command: &mut Command,
    ) -> Result<(), InjectError>;
}

#[derive(Debug)]
pub struct InjectError {
    pub detail: String,
}

impl fmt::Display for InjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Injection error: {}", self.detail)
    }
}

impl std::error::Error for InjectError {}
