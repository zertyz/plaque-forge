//! Replaceable infrastructure contracts shared by production workflows and tests.
//!
//! Keep this module deliberately small. A dependency belongs here only when it is an
//! external/process boundary with an independent reason to vary or a material test cost.

use std::{ffi::OsString, path::Path, process::Command};

use anyhow::{Context, Result};

/// Portable result of a non-interactive child process.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Portable completion result for a child process whose output is inherited or irrelevant.
#[derive(Debug, Clone, Copy)]
pub struct CommandStatus {
    pub success: bool,
    pub code: Option<i32>,
}

/// Executes an external command whose complete output can be collected.
///
/// Streaming video encode/decode uses a different lifecycle and intentionally does not
/// pretend to fit this contract.
pub trait CommandExecutor {
    fn output(&self, program: &Path, args: &[OsString]) -> Result<CommandOutput>;
    fn status(&self, program: &Path, args: &[OsString]) -> Result<CommandStatus>;
}

/// Production command executor backed by `std::process::Command`.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsCommandExecutor;

/// Shared production command executor used by the default application services.
pub static OS_COMMAND_EXECUTOR: OsCommandExecutor = OsCommandExecutor;

impl CommandExecutor for OsCommandExecutor {
    fn output(&self, program: &Path, args: &[OsString]) -> Result<CommandOutput> {
        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("failed to launch {}", program.display()))?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    fn status(&self, program: &Path, args: &[OsString]) -> Result<CommandStatus> {
        let status = Command::new(program)
            .args(args)
            .status()
            .with_context(|| format!("failed to launch {}", program.display()))?;
        Ok(CommandStatus {
            success: status.success(),
            code: status.code(),
        })
    }
}
