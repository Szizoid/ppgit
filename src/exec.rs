use std::ffi::{OsStr, OsString};
use std::io;
use std::process::{Command, ExitCode, ExitStatus, Output};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

/// Runs `program <args>`, inheriting stdio — the terminal is passed
/// through, so this (and everything built on it) is the only family safe
/// for interactive git commands (editors, pagers, credential prompts).
/// Returns the raw `ExitStatus`, unexamined. Generic over the same bound
/// `Command::args` itself uses, so this covers both ppgit's own fixed
/// `&[&str]` invocations and passing through arbitrary (possibly
/// non-UTF-8) `OsString` user arguments unchanged.
pub fn run_loud<I, S>(program: &str, args: I) -> io::Result<ExitStatus>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(program).args(args).status()
}

/// `run_loud`, but `Ok(())` only if the program was launched *and* exited
/// successfully — any other outcome becomes the `ExitCode` a top-level
/// command (e.g. `cmd_init`) should return.
pub fn run_loud_checked(program: &str, args: &[&str]) -> Result<(), ExitCode> {
    match run_loud(program, args) {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(status_to_exit_code(&status)),
        Err(e) => {
            eprintln!("ppgit: failed to launch {program}: {e}");
            Err(ExitCode::FAILURE)
        }
    }
}

/// Runs `program <args>` with stdout/stderr piped (captured, not shown)
/// and stdin closed — i.e. *not* interactive-safe, never use this for
/// anything that might read from the terminal or prompt for input. Use
/// `run_quiet_ok`/`run_quiet_stdout` below unless you need the full
/// `Output` (exit status *and* stdout *and* stderr) yourself.
fn run_quiet(program: &str, args: &[&str]) -> io::Result<Output> {
    Command::new(program).args(args).output()
}

/// Quietly runs `program <args>` and reports whether it both launched and
/// exited successfully — for checks like "does this repo already exist?",
/// where only a yes/no is needed and nothing should be printed either way.
pub fn run_quiet_ok(program: &str, args: &[&str]) -> bool {
    run_quiet(program, args).is_ok_and(|output| output.status.success())
}

/// Quietly runs `program <args>` and returns its stdout as a trimmed
/// string. Errors (rather than returning partial/garbage output) if the
/// program couldn't be launched or exited unsuccessfully.
pub fn run_quiet_stdout(program: &str, args: &[&str]) -> io::Result<String> {
    let output = run_quiet(program, args)?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{program} {} exited with {}",
            args.join(" "),
            output.status,
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// A process's exit code, if it has one, as an `ExitCode` — `FAILURE` if it
/// was killed by a signal instead.
fn status_to_exit_code(status: &ExitStatus) -> ExitCode {
    match status.code() {
        Some(code) => ExitCode::from(code as u8),
        None => ExitCode::FAILURE,
    }
}

/// Turns any `io::Result` into `Result<T, ExitCode>`, printing
/// "ppgit: failed to {context}: {error}" on failure. The filesystem/other-IO
/// counterpart to `run_loud_checked` — for the many "do this or bail out of
/// the whole command with a message" steps a command like `cmd_init` is
/// made of.
pub fn io_checked<T>(result: io::Result<T>, context: &str) -> Result<T, ExitCode> {
    result.map_err(|e| {
        eprintln!("ppgit: failed to {context}: {e}");
        ExitCode::FAILURE
    })
}

/// Interprets a `run_loud`-obtained `ExitStatus` as an `ExitCode`, on the
/// way out reporting anything that isn't a plain exit (killed by a signal,
/// or no exit code at all — both otherwise-silent outcomes the user would
/// have no other explanation for).
trait IntoExitCode {
    fn into_exit_code(self, program: &str) -> ExitCode;
}

impl IntoExitCode for ExitStatus {
    fn into_exit_code(self, program: &str) -> ExitCode {
        if self.success() {
            return ExitCode::SUCCESS;
        }
        if self.code().is_some() {
            return status_to_exit_code(&self);
        }

        #[cfg(unix)]
        if let Some(signal) = self.signal() {
            eprintln!("ppgit: {program} was terminated by signal {signal}");
            return ExitCode::from(128 + signal as u8);
        }

        eprintln!("ppgit: {program} terminated abnormally, no exit code available");
        ExitCode::FAILURE
    }
}

/// The passthrough path: forwards `args` to `git` as-is, interactively —
/// this is the one place ppgit runs an entirely user-controlled command
/// line, so it must stay on the `run_loud` family no matter what.
pub fn to_git(args: &[OsString]) -> ExitCode {
    match run_loud("git", args) {
        Ok(status) => status.into_exit_code("git"),
        Err(e) => {
            eprintln!("ppgit: failed to launch git: {e}");
            ExitCode::FAILURE
        }
    }
}
