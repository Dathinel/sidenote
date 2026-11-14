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
    pub fn spawn(&self, command: &Command) -> io::Result<JobProcess> {
        if !Path::new(command.get_program()).is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Absolute worker path required",
            ));
        }
        let (input, writer) = std::io::pipe()?;
        let output = File::options().write(true).open("NUL")?;
        let error = output.try_clone()?;
        for h in [
            input.as_raw_handle(),
            output.as_raw_handle(),
            error.as_raw_handle(),
        ] {
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
            [input.as_handle(), output.as_handle(), error.as_handle()],
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
        match unsafe { WaitForSingleObject(self.handle.as_raw_handle(), milliseconds) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.handle.as_raw_handle(), &mut code) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(Some(ExitStatus::from_raw(code)))
            }
            _ => Err(io::Error::last_os_error()),
        }
    }
}
