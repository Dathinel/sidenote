use super::*;
use std::{
    fs::File,
    os::windows::{
        io::{AsHandle, AsRawHandle, OwnedHandle},
        process::ExitStatusExt,
    },
    process::{Command, ExitStatus},
};
use windows_sys::Win32::{
    Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::*,
};
pub struct JobProcess {
    pub(crate) handle: OwnedHandle,
    pub input: std::io::PipeWriter,
}
impl ProcessJob {
    #[cfg(feature = "asr-runtime")]
    pub(crate) fn contains_pid(&self, pid: u32) -> io::Result<bool> {
        use windows_sys::Win32::System::JobObjects::IsProcessInJob;
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        use std::os::windows::io::FromRawHandle;
        let process = unsafe { OwnedHandle::from_raw_handle(handle) };
        let mut inside = 0;
        if unsafe { IsProcessInJob(process.as_raw_handle(), self.0.as_raw_handle(), &mut inside) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(inside != 0)
    }
    pub fn spawn(&self, command: &Command) -> io::Result<JobProcess> {
        self.spawn_output(command, None)
    }
    pub(crate) fn spawn_output(
        &self,
        command: &Command,
        stdout: Option<&std::io::PipeWriter>,
    ) -> io::Result<JobProcess> {
        if !Path::new(command.get_program()).is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Absolute worker path required",
            ));
        }
        let (input, writer) = std::io::pipe()?;
        let output = File::options().write(true).open("NUL")?;
        let error = output.try_clone()?;
        let stdout = stdout.map_or(output.as_raw_handle(), AsRawHandle::as_raw_handle);
        for h in [input.as_raw_handle(), stdout, error.as_raw_handle()] {
            if unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        // Only explicit entries survive; the caller must construct its allowlist.
        let mut environment = Vec::new();
        for (key, value) in command.get_envs().filter_map(|(k, v)| v.map(|v| (k, v))) {
            environment.extend(key.encode_wide());
            environment.push(61);
            environment.extend(value.encode_wide());
            environment.push(0);
        }
        environment.push(0);
        let directory = command.get_current_dir().map(|p| wide(p.as_os_str()));
        let (handle, thread) = sidenote_sync::windows_launch::spawn(
            &wide(command.get_program()),
            &mut sidenote_sync::windows_launch::command_line(command)?,
            &environment,
            [
                input.as_handle(),
                unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(stdout) },
                error.as_handle(),
            ],
            directory.as_deref(),
            CREATE_NO_WINDOW,
            Some(self.0.as_handle()),
        )?;
        drop(thread);
        Ok(JobProcess {
            handle,
            input: writer,
        })
    }
}
impl JobProcess {
    pub fn id(&self) -> u32 {
        unsafe { GetProcessId(self.handle.as_raw_handle()) }
    }
    pub fn kill(&self) -> io::Result<()> {
        if unsafe { TerminateProcess(self.handle.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        self.wait_for(0)
    }
    pub fn wait(&self) -> io::Result<ExitStatus> {
        self.wait_for(INFINITE)?
            .ok_or_else(|| io::Error::other("Process wait timed out"))
    }
    pub fn wait_for(&self, milliseconds: u32) -> io::Result<Option<ExitStatus>> {
        Self::wait_handle(&self.handle, milliseconds)
    }
    pub(crate) fn wait_handle(
        handle: &OwnedHandle,
        milliseconds: u32,
    ) -> io::Result<Option<ExitStatus>> {
        match unsafe { WaitForSingleObject(handle.as_raw_handle(), milliseconds) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(handle.as_raw_handle(), &mut code) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(Some(ExitStatus::from_raw(code)))
            }
            _ => Err(io::Error::last_os_error()),
        }
    }
}
