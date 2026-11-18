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
    use crate::{LocalPtyCommandMode, LocalPtySpawnConfig, spawn_local_pty};
    use std::{
        fs,
        io::Write,
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };

    #[test]
    fn closing_pty_terminates_shell_and_detached_descendants() {
        let directory = std::env::temp_dir().join(format!("codux-pty-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let pid_file = directory.join("pids.txt");
        fs::write(
            directory.join("child.ps1"),
            "$grandchild = Start-Process powershell.exe '-NoProfile -Command Start-Sleep -Seconds 90' -WindowStyle Hidden -PassThru\n\
             Set-Content -LiteralPath $env:CODUX_TEST_PID_FILE -Value @($PID, $grandchild.Id)\n\
             Start-Sleep -Seconds 90\n",
        ).unwrap();
        let config = LocalPtySpawnConfig {
            shell: "cmd.exe".to_owned(),
            cwd: None,
            initial_command: None,
            cols: 80,
            rows: 24,
            env: [
                (
                    "CODUX_TEST_CHILD_SCRIPT".to_owned(),
                    directory.join("child.ps1").to_string_lossy().into_owned(),
                ),
                (
                    "CODUX_TEST_PID_FILE".to_owned(),
                    pid_file.to_string_lossy().into_owned(),
                ),
            ]
            .into(),
            clear_env: false,
            command_mode: LocalPtyCommandMode::Default,
        };
        let mut terminal = spawn_local_pty(config.clone()).unwrap();
        let other_terminal = spawn_local_pty(config).unwrap();
        terminal.writer.write_all(b"start \"\" /b powershell.exe -NoProfile -ExecutionPolicy Bypass -File \"%CODUX_TEST_CHILD_SCRIPT%\"\r\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(15);
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
        let handles = std::iter::once(terminal.control.process_id().unwrap())
            .chain(descendants)
            .map(process_handle)
            .collect::<Vec<_>>();
        let other_handle = process_handle(other_terminal.control.process_id().unwrap());
        terminal.control.kill().unwrap();
        assert!(terminal.control.wait_exit_code().is_some());
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
        other_terminal.control.kill().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }
}
