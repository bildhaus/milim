//! Helpers for spawning and containing child processes.
//!
//! On Windows, a GUI-subsystem app (the Tauri desktop shell and its embedded
//! server) that spawns a console executable — `git`, `node`, `cmd`,
//! `docker`, MCP servers, and other helpers — briefly pops a console
//! window for each spawn unless `CREATE_NO_WINDOW` is set. These helpers apply
//! that flag and are no-ops on other platforms.

/// Windows [`CREATE_NO_WINDOW`] process-creation flag.
///
/// [`CREATE_NO_WINDOW`]: https://learn.microsoft.com/windows/win32/procthread/process-creation-flags
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Keeps one Milim-owned child process tree contained for its full lifetime.
pub struct ProcessTreeGuard {
    #[cfg(windows)]
    job: Option<std::os::windows::io::OwnedHandle>,
    #[cfg(unix)]
    process_group: Option<u32>,
}

impl ProcessTreeGuard {
    /// Attach a newly spawned child. Unix commands must use `process_group(0)`
    /// before spawning so descendants share the child's process group.
    pub fn attach(pid: u32) -> std::io::Result<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
            use std::ptr;
            use windows_sys::Win32::System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            };
            use windows_sys::Win32::System::Threading::{
                OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
            };

            let raw_job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if raw_job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let job = unsafe { OwnedHandle::from_raw_handle(raw_job as _) };
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if unsafe {
                SetInformationJobObject(
                    job.as_raw_handle() as _,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of_val(&limits) as u32,
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            let raw_process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
            if raw_process.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let process = unsafe { OwnedHandle::from_raw_handle(raw_process as _) };
            if unsafe {
                AssignProcessToJobObject(job.as_raw_handle() as _, process.as_raw_handle() as _)
            } == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Self { job: Some(job) })
        }

        #[cfg(unix)]
        {
            Ok(Self {
                process_group: Some(pid),
            })
        }

        #[cfg(not(any(windows, unix)))]
        {
            Ok(Self {})
        }
    }

    pub fn terminate(&mut self) {
        #[cfg(windows)]
        drop(self.job.take());

        #[cfg(unix)]
        if let Some(pid) = self.process_group.take() {
            let group = format!("-{pid}");
            let _ = std::process::Command::new("kill")
                .args(["-KILL", &group])
                .status();
        }
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

/// Apply [`CREATE_NO_WINDOW`] to a [`std::process::Command`] on Windows so the
/// spawned console program does not flash a window. No-op elsewhere.
pub fn hide_console(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
