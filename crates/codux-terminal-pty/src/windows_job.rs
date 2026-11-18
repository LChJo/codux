use portable_pty::Child;
use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};

pub(super) struct TerminalJob(OwnedHandle);

impl TerminalJob {
    pub(super) fn new() -> io::Result<Self> {
        // The handle is unnamed and non-inheritable, so it closes on host exit.
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                ptr::from_ref(&limits).cast(),
                size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }

    pub(super) fn attach(
        &self,
        mut child: Box<dyn Child + Send + Sync>,
    ) -> io::Result<Box<dyn Child + Send + Sync>> {
        let error = match child.as_raw_handle() {
            Some(process) => {
                if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), process) } != 0 {
                    return Ok(child);
                }
                io::Error::last_os_error()
            }
            None => io::Error::other("PTY shell has no Windows process handle"),
        };
        // A short-lived shell can finish before portable-pty returns its handle.
        if matches!(child.try_wait(), Ok(Some(_))) {
            return Ok(child);
        }
        let _ = child.clone_killer().kill();
        Err(error)
    }

    pub(super) fn terminate(&self) -> io::Result<()> {
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LocalPtyCommandMode, LocalPtyProcessHandle, LocalPtySpawnConfig, spawn_local_pty};
    use codux_terminal_core::{HeadlessTerminalScreen, TerminalPtyResponder};
    use std::{
        fs,
        io::{Read, Write},
        os::windows::process::CommandExt,
        path::PathBuf,
        process::{self, Command, Stdio},
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{
            DETACHED_PROCESS, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
        },
    };

    const FIXTURE: &str = "windows_job::tests::pty_process_tree_fixture";
    const FIXTURE_DIRECTORY: &str = "CODUX_TEST_PTY_FIXTURE_DIRECTORY";
    const FIXTURE_ROLE: &str = "CODUX_TEST_PTY_FIXTURE_ROLE";

    fn spawn_detached_fixture(role: &str) -> io::Result<process::Child> {
        Command::new(std::env::current_exe()?)
            .arg(FIXTURE)
            .env(FIXTURE_ROLE, role)
            .creation_flags(DETACHED_PROCESS)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }

    fn spawn_fixture_terminal(config: LocalPtySpawnConfig) -> LocalPtyProcessHandle {
        let cols = usize::from(config.cols);
        let rows = usize::from(config.rows);
        let mut process = spawn_local_pty(config).unwrap();
        // ConPTY may request the cursor position before running the fixture.
        let writer = parking_lot::Mutex::new(process.writer);
        let responder: TerminalPtyResponder = Arc::new(move |reply| {
            let mut writer = writer.lock();
            let _ = writer.write_all(reply);
            let _ = writer.flush();
        });
        thread::spawn(move || {
            let mut screen =
                HeadlessTerminalScreen::new_with_responder(cols, rows, 0, Some(responder));
            let mut bytes = [0; 4096];
            while let Ok(count) = process.reader.read(&mut bytes) {
                if count == 0 {
                    break;
                }
                screen.process(&bytes[..count]);
            }
        });
        process.control
    }

    #[test]
    fn pty_process_tree_fixture() {
        // The regression test runs this fixture in separate OS processes.
        let Some(directory) = std::env::var_os(FIXTURE_DIRECTORY).map(PathBuf::from) else {
            return;
        };
        let role = std::env::var(FIXTURE_ROLE).unwrap();
        let result = match role.as_str() {
            "shell" => {
                let start = directory.join("start");
                while !start.is_file() {
                    thread::sleep(Duration::from_millis(25));
                }
                spawn_detached_fixture("child").map(|_| ())
            }
            "child" => spawn_detached_fixture("idle").and_then(|grandchild| {
                fs::write(
                    directory.join("pids.txt"),
                    format!("{}\n{}\n", process::id(), grandchild.id()),
                )
            }),
            "idle" => Ok(()),
            _ => panic!("unknown PTY test fixture role: {role}"),
        };
        if let Err(error) = result {
            let _ = fs::write(directory.join("fixture-error.txt"), error.to_string());
            panic!("could not prepare PTY fixture ({role}): {error}");
        }
        thread::sleep(Duration::from_secs(90));
    }

    #[test]
    fn closing_pty_terminates_shell_and_detached_descendants() {
        let directory = std::env::temp_dir().join(format!("codux-pty-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let pid_file = directory.join("pids.txt");
        let error_file = directory.join("fixture-error.txt");
        let mut config = LocalPtySpawnConfig {
            shell: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            cwd: None,
            initial_command: Some(FIXTURE.into()),
            cols: 80,
            rows: 24,
            env: [
                (
                    FIXTURE_DIRECTORY.into(),
                    directory.to_string_lossy().into_owned(),
                ),
                (FIXTURE_ROLE.into(), "idle".into()),
            ]
            .into(),
            clear_env: false,
            command_mode: LocalPtyCommandMode::InteractiveLogin,
        };
        let other_terminal = spawn_fixture_terminal(config.clone());
        config.env.insert(FIXTURE_ROLE.into(), "shell".into());
        let terminal = spawn_fixture_terminal(config);
        // Release the fixture only after the PTY shell has joined its job.
        fs::write(directory.join("start"), []).unwrap();

        let deadline = Instant::now() + Duration::from_secs(30);
        let descendants = loop {
            if let Ok(contents) = fs::read_to_string(&pid_file) {
                let pids = contents
                    .lines()
                    .filter_map(|pid| pid.trim().parse::<u32>().ok())
                    .collect::<Vec<_>>();
                if pids.len() == 2 {
                    break pids;
                }
            }
            if let Ok(error) = fs::read_to_string(&error_file) {
                panic!("could not start PTY descendants: {error}");
            }
            assert!(
                Instant::now() < deadline,
                "shell did not start both descendants"
            );
            thread::sleep(Duration::from_millis(25));
        };
        let process_handle = |pid| {
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            assert!(!handle.is_null(), "could not open process {pid}");
            unsafe { OwnedHandle::from_raw_handle(handle) }
        };
        let handles = std::iter::once(terminal.process_id().unwrap())
            .chain(descendants)
            .map(process_handle)
            .collect::<Vec<_>>();
        let other_handle = process_handle(other_terminal.process_id().unwrap());
        for handle in &handles {
            assert_eq!(
                unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) },
                WAIT_TIMEOUT
            );
        }
        terminal.kill().unwrap();
        assert!(terminal.wait_exit_code().is_some());
        for handle in handles {
            assert_eq!(
                unsafe { WaitForSingleObject(handle.as_raw_handle(), 5_000) },
                WAIT_OBJECT_0
            );
        }
        assert_eq!(
            unsafe { WaitForSingleObject(other_handle.as_raw_handle(), 0) },
            WAIT_TIMEOUT
        );
        other_terminal.kill().unwrap();
        assert!(other_terminal.wait_exit_code().is_some());
        fs::remove_dir_all(directory).unwrap();
    }
}
