//! Shared Windows startup attributes and raw process creation; job policy stays with callers.
use std::{
    io,
    os::windows::io::{AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::System::Threading::*;
fn check(value: i32) -> io::Result<()> {
    if value == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
struct Attributes(Vec<usize>);
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast());
        }
    }
}
pub fn spawn(
    app: &[u16],
    command: &mut [u16],
    environment: &[u16],
    handles: [BorrowedHandle<'_>; 3],
    directory: Option<&[u16]>,
    flags: u32,
) -> io::Result<(OwnedHandle, OwnedHandle)> {
    if app.last() != Some(&0)
        || command.last() != Some(&0)
        || !environment.ends_with(&[0, 0])
        || directory.is_some_and(|d| d.last() != Some(&0))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unterminated launch data",
        ));
    }
    let handles = handles.map(|h| h.as_raw_handle());
    let mut size = 0;
    unsafe {
        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size);
    }
    if size == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut storage = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
    check(unsafe {
        InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 1, 0, &mut size)
    })?;
    let mut attributes = Attributes(storage);
    let list = attributes.0.as_mut_ptr().cast();
    check(unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_ptr().cast(),
            std::mem::size_of_val(&handles),
            ptr::null_mut(),
            ptr::null(),
        )
    })?;
    let startup = STARTUPINFOEXW {
        StartupInfo: STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOEXW>() as u32,
            dwFlags: STARTF_USESTDHANDLES,
            hStdInput: handles[0],
            hStdOutput: handles[1],
            hStdError: handles[2],
            ..Default::default()
        },
        lpAttributeList: list,
    };
    let mut process = PROCESS_INFORMATION::default();
    check(unsafe {
        CreateProcessW(
            app.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            flags | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            environment.as_ptr().cast(),
            directory.map_or(ptr::null(), |d| d.as_ptr()),
            &startup.StartupInfo,
            &mut process,
        )
    })?;
    Ok(unsafe {
        (
            OwnedHandle::from_raw_handle(process.hProcess),
            OwnedHandle::from_raw_handle(process.hThread),
        )
    })
}
