//! Shared I/O helpers.
//!
//! Centralizes tolerated `BrokenPipe` handling so `| head` and similar
//! downstream closures do not turn a successful listing into a failure.
//! Both human and machine consumers benefit from the same contract.

use std::io::Write;

/// Write one line to `out`, tolerating a closed downstream pipe.
///
/// Many `plaque-forge list` invocations are piped to `head`/`grep`. When the
/// reader closes early the kernel returns `BrokenPipe` on the next write.
/// That is not a program error: the requested prefix was already delivered.
pub(crate) fn write_stdout_line(out: &mut impl Write, text: &str) -> anyhow::Result<()> {
    match writeln!(out, "{text}") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Flush `out`, tolerating `BrokenPipe` for the same reason as
/// [`write_stdout_line`].
pub(crate) fn flush_tolerating_broken_pipe(out: &mut impl Write) -> anyhow::Result<()> {
    match out.flush() {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error.into()),
    }
}
