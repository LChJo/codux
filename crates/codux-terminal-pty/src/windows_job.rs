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
        fs, thread,
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
        let error_file = directory.join("fixture-error.txt");
        let script = directory.join("fixture.ps1");
        fs::write(
            &script,
            r#"param([switch]$Descendant)
$ErrorActionPreference = 'Stop'
$powershell = Join-Path $PSHOME 'powershell.exe'
try {
    if ($Descendant) {
        $grandchild = Start-Process -FilePath $powershell -ArgumentList '-NoProfile -Command Start-Sleep -Seconds 90' -WindowStyle Hidden -PassThru
        Set-Content -LiteralPath (Join-Path $PSScriptRoot 'pids.txt') -Value @($PID, $grandchild.Id)
    } else {
        $start = Join-Path $PSScriptRoot 'start'
        while (-not (Test-Path -LiteralPath $start)) { Start-Sleep -Milliseconds 25 }
        $arguments = '-NoProfile -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" -Descendant'
        Start-Process -FilePath $powershell -ArgumentList $arguments -WindowStyle Hidden
    }
    Start-Sleep -Seconds 90
} catch {
    Set-Content -LiteralPath (Join-Path $PSScriptRoot 'fixture-error.txt') -Encoding UTF8 -Value ($_ | Out-String)
    exit 1
}
"#,
        )
        .unwrap();
        let other_config = LocalPtySpawnConfig {
            shell: "cmd.exe".into(),
            cwd: None,
            initial_command: None,
            cols: 80,
            rows: 24,
            env: Default::default(),
            clear_env: false,
            command_mode: LocalPtyCommandMode::Default,
        };
        let other_terminal = spawn_local_pty(other_config.clone()).unwrap();
        let terminal = spawn_local_pty(LocalPtySpawnConfig {
            shell: "powershell.exe".into(),
            initial_command: Some("& $env:CODUX_TEST_PTY_SCRIPT".into()),
            env: [
                ("DMUX_PS_HOOK_SCRIPT".into(), String::new()),
                (
                    "CODUX_TEST_PTY_SCRIPT".into(),
                    script.to_string_lossy().into_owned(),
                ),
            ]
            .into(),
            command_mode: LocalPtyCommandMode::InteractiveLogin,
            ..other_config
        })
        .unwrap();
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
        assert!(other_terminal.control.wait_exit_code().is_some());
        fs::remove_dir_all(directory).unwrap();
    }
}
