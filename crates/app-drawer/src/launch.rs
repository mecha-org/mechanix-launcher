//! Starting an app detached from the launcher.

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

/// Runs `argv` in its own process group with the launcher's environment
/// and no stdio. A thread reaps the child so it never lingers as a zombie.
pub(crate) fn spawn(argv: &[String]) -> io::Result<()> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty command"))?;
    let mut child = Command::new(program)
        .args(args)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawns_or_reports() {
        assert!(spawn(&["true".to_string()]).is_ok());
        assert!(spawn(&[]).is_err());
        assert!(spawn(&["/nonexistent/app-drawer-test".to_string()]).is_err());
    }
}
