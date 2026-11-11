use std::{
    io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow},
};
fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}
pub fn show_error(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    let title = wide(std::ffi::OsStr::new("Sidenote"));
    let text = wide(std::ffi::OsStr::new(message));
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
pub fn process_running(pid: u32) -> bool {
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return GetLastError() != 87;
        }
        let mut code = 0;
        let running = GetExitCodeProcess(handle, &mut code) == 0 || code == 259;
        CloseHandle(handle);
        running
    }
}
fn processes() -> io::Result<Vec<(u32, u32, String)>> {
    use windows_sys::Win32::{Foundation::INVALID_HANDLE_VALUE, System::Diagnostics::ToolHelp::*};
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        let mut next = Process32FirstW(snapshot, &mut entry);
        let mut found = Vec::new();
        while next != 0 {
            let end = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            found.push((
                entry.th32ProcessID,
                entry.th32ParentProcessID,
                String::from_utf16_lossy(&entry.szExeFile[..end]),
            ));
            next = Process32NextW(snapshot, &mut entry);
        }
        let error = GetLastError();
        CloseHandle(snapshot);
        if error != 18 {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        Ok(found)
    }
}
pub fn hex_running() -> io::Result<bool> {
    Ok(processes()?.iter().any(|(_, _, name)| {
        name.eq_ignore_ascii_case("hex.exe") || name.eq_ignore_ascii_case("hex.com")
    }))
}
pub fn updater_child() -> io::Result<Option<u32>> {
    Ok(processes()?
        .iter()
        .find(|(_, parent, name)| {
            *parent == std::process::id() && name.eq_ignore_ascii_case("Update.exe")
        })
        .map(|(pid, _, _)| *pid))
}
pub fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    move_file(from, to, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
}
/// Durable publication of an immutable file. An existing destination is never replaced.
pub fn publish_file(from: &Path, to: &Path) -> io::Result<()> {
    move_file(from, to, MOVEFILE_WRITE_THROUGH)
}
fn rename_path(path: &Path) -> io::Result<PathBuf> {
    let path = std::path::absolute(path)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing file name"))?;
    // Canonical parents give Win32 verbatim paths without requiring the destination to exist.
    Ok(path.parent().unwrap().canonicalize()?.join(name))
}
fn move_file(from: &Path, to: &Path, flags: u32) -> io::Result<()> {
    let from = rename_path(from)?;
    let to = rename_path(to)?;
    crate::core::store_io::retry_sharing_violation(|| {
        if unsafe {
            MoveFileExW(
                wide(from.as_os_str()).as_ptr(),
                wide(to.as_os_str()).as_ptr(),
                flags,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            // MoveFileEx can report access denied for a locked destination. Confirm sharing before retrying.
            if error.raw_os_error() == Some(5) {
                for path in [&to, &from] {
                    if let Err(sharing) = std::fs::File::open(path)
                        && matches!(sharing.raw_os_error(), Some(32 | 33))
                    {
                        return Err(sharing);
                    }
                }
            }
            Err(error)
        } else {
            Ok(())
        }
    })
}
pub struct Instance(HANDLE);
/// All runner children die when their owning job closes, including on timeout.
pub struct ProcessJob(HANDLE);
impl ProcessJob {
    pub fn attach(child: &std::process::Child) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Self(handle);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle() as HANDLE) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }
    }
}
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub fn single_instance() -> io::Result<Option<Instance>> {
    // Share the old mutex so an existing install and Sidenote cannot open competing windows.
    let name = wide(std::ffi::OsStr::new("Local\\HexMeetingsWindow"));
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(handle);
        }
        // The first window may still be starting when a second launch arrives.
        for _ in 0..40 {
            let title = wide(std::ffi::OsStr::new("Sidenote"));
            let mut hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
            if hwnd.is_null() {
                let legacy = wide(std::ffi::OsStr::new("Meetings"));
                hwnd = unsafe { FindWindowW(std::ptr::null(), legacy.as_ptr()) };
            }
            if !hwnd.is_null() {
                unsafe {
                    ShowWindow(hwnd, SW_RESTORE);
                    SetForegroundWindow(hwnd);
                }
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Ok(None)
    } else {
        Ok(Some(Instance(handle)))
    }
}
/// Gives the window the icon embedded in the exe (resource ID 1, see build.rs) at the title bar
/// and Alt+Tab sizes. Rune has no window-icon option, and without this the title bar shows the
/// generic icon. Does nothing when the exe has no icon resource, such as an example binary.
pub fn set_window_icon(hwnd: isize) {
    use windows_sys::Win32::{
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            WindowsAndMessaging::{
                ICON_BIG, ICON_SMALL, IMAGE_ICON, LoadImageW, SM_CXICON, SM_CXSMICON, SM_CYICON,
                SM_CYSMICON, SendMessageW, WM_SETICON,
            },
        },
    };
    unsafe {
        let hwnd = hwnd as *mut std::ffi::c_void;
        let module = GetModuleHandleW(std::ptr::null());
        let dpi = match GetDpiForWindow(hwnd) {
            0 => 96,
            dpi => dpi,
        };
        for (kind, cx, cy) in [
            (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
            (ICON_BIG, SM_CXICON, SM_CYICON),
        ] {
            let icon = LoadImageW(
                module,
                // MAKEINTRESOURCE(1). Not ptr::dangling(): for u16 that is address 2, the wrong resource.
                std::ptr::without_provenance::<u16>(1),
                IMAGE_ICON,
                GetSystemMetricsForDpi(cx, dpi),
                GetSystemMetricsForDpi(cy, dpi),
                0,
            );
            if !icon.is_null() {
                SendMessageW(hwnd, WM_SETICON, kind as usize, icon as isize);
            }
        }
    }
}
/// The client size, in points, that keeps a window of about `want` points inside the work area
/// of the monitor `hwnd` is on, leaving room for the title bar, the frame and a margin.
pub fn fit_to_work_area(hwnd: isize, want: (f32, f32)) -> (f32, f32) {
    use windows_sys::Win32::{
        Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        },
        UI::HiDpi::GetDpiForWindow,
    };
    unsafe {
        let hwnd = hwnd as *mut std::ffi::c_void;
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if monitor.is_null() || GetMonitorInfoW(monitor, &mut info) == 0 {
            return want;
        }
        let dpi = GetDpiForWindow(hwnd);
        let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
        let area_w = (info.rcWork.right - info.rcWork.left) as f32 / scale;
        let area_h = (info.rcWork.bottom - info.rcWork.top) as f32 / scale;
        (
            want.0.min(area_w - 32.0).max(360.0),
            want.1.min(area_h - 80.0).max(480.0),
        )
    }
}

/// Opens `folder` in File Explorer. It must be an existing folder. The path goes to
/// `explorer.exe` as one argument and never through a shell, so no name in it can run anything.
pub fn open_folder(folder: &Path) -> io::Result<()> {
    if !std::fs::metadata(folder)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "that is not a folder",
        ));
    }
    std::process::Command::new("explorer.exe")
        .arg(folder)
        .spawn()
        .map(|_| ())
}

/// Opens File Explorer on `file`'s folder with the file selected. It must be an existing file.
pub fn reveal_file(file: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    if !std::fs::metadata(file)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "that is not a file",
        ));
    }
    // A file name cannot hold a quote on Windows, so the quoted form below cannot be broken
    // out of. `raw_arg` keeps the quotes explorer needs for a path with spaces.
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", file.display()))
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn long_paths_keep_publication_and_replacement_semantics() {
        let root = std::env::temp_dir().join(format!(
            "sidenote-rename-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let folder = root.join("a".repeat(100)).join("b".repeat(100));
        fs::create_dir_all(&folder).unwrap();
        let from = folder.join("source.tmp");
        let to = folder.join("target.json");
        assert!(from.as_os_str().encode_wide().count() > 260);
        fs::write(&from, b"first").unwrap();
        let result = (|| -> io::Result<()> {
            publish_file(&from, &to)?;
            assert_eq!(fs::read(&to)?, b"first");
            fs::write(&from, b"second")?;
            assert!(publish_file(&from, &to).is_err());
            assert_eq!(fs::read(&to)?, b"first");
            assert_eq!(fs::read(&from)?, b"second");
            replace_file(&from, &to)?;
            assert_eq!(fs::read(&to)?, b"second");
            assert!(!from.exists());
            Ok(())
        })();
        assert!(root.starts_with(std::env::temp_dir()) && root != std::env::temp_dir());
        fs::remove_dir_all(&root).unwrap();
        result.unwrap();
    }
}

/// Rune's pinned close hook cannot veto a close. Catch WM_CLOSE before winit sees it,
/// then queue the save decision on Rune's UI thread.
pub fn guard_window_close(hwnd: isize, request: impl Fn() + 'static) -> io::Result<()> {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{WM_CLOSE, WM_NCDESTROY},
        },
    };
    type Request = Box<dyn Fn()>;
    unsafe extern "system" fn procedure(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        if msg == WM_CLOSE {
            unsafe {
                (&*(data as *const Request))();
            }
            return 0;
        }
        if msg == WM_NCDESTROY {
            unsafe {
                RemoveWindowSubclass(hwnd, Some(procedure), id);
                drop(Box::from_raw(data as *mut Request));
            }
        }
        unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
    }
    let request: Box<Request> = Box::new(Box::new(request));
    let data = Box::into_raw(request);
    if unsafe { SetWindowSubclass(hwnd as _, Some(procedure), 1, data as usize) } == 0 {
        unsafe {
            drop(Box::from_raw(data));
        }
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod close_tests {
    use super::*;
    #[test]
    fn close_request_is_intercepted_before_windows_destroys_the_window() {
        use std::{cell::Cell, rc::Rc};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, HWND_MESSAGE, IsWindow, SendMessageW, WM_CLOSE,
        };
        let requested = Rc::new(Cell::new(0));
        let destroyed = Rc::new(Cell::new(false));
        struct Request {
            calls: Rc<Cell<u32>>,
            destroyed: Rc<Cell<bool>>,
        }
        impl Drop for Request {
            fn drop(&mut self) {
                self.destroyed.set(true);
            }
        }
        let request = Request {
            calls: requested.clone(),
            destroyed: destroyed.clone(),
        };
        let class = wide(std::ffi::OsStr::new("STATIC"));
        // A message-only window has no visible UI and runs no application code.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!hwnd.is_null());
        guard_window_close(hwnd as isize, move || {
            request.calls.set(request.calls.get() + 1);
            let _ = &request;
        })
        .unwrap();
        for count in 1..=2 {
            unsafe {
                SendMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            assert_eq!(requested.get(), count);
            assert_ne!(unsafe { IsWindow(hwnd) }, 0);
        }
        assert_ne!(unsafe { DestroyWindow(hwnd) }, 0);
        assert!(destroyed.get());
    }
}

/// Called only by a person's Share action, never by offscreen proofs.
pub fn save_html_dialog(default_name: &str) -> io::Result<Option<PathBuf>> {
    use windows_sys::Win32::UI::{Controls::Dialogs::*, Input::KeyboardAndMouse::GetActiveWindow};
    let mut buffer = vec![0u16; 32768];
    let name = wide(std::ffi::OsStr::new(default_name));
    if name.len() > buffer.len() {
        return Err(io::Error::other("The file name is too long."));
    }
    buffer[..name.len()].copy_from_slice(&name);
    let filter: Vec<u16> = "HTML page (*.html)\0*.html\0\0".encode_utf16().collect();
    let extension = wide(std::ffi::OsStr::new("html"));
    let title = wide(std::ffi::OsStr::new("Share annotation note"));
    unsafe {
        let mut dialog: OPENFILENAMEW = std::mem::zeroed();
        dialog.lStructSize = std::mem::size_of_val(&dialog) as u32;
        dialog.hwndOwner = GetActiveWindow();
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = buffer.as_mut_ptr();
        dialog.nMaxFile = buffer.len() as u32;
        dialog.lpstrDefExt = extension.as_ptr();
        dialog.lpstrTitle = title.as_ptr();
        dialog.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
        if GetSaveFileNameW(&mut dialog) == 0 {
            let error = CommDlgExtendedError();
            return if error == 0 {
                Ok(None)
            } else {
                Err(io::Error::other("The save dialog could not open."))
            };
        }
    }
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    use std::os::windows::ffi::OsStringExt;
    Ok(Some(std::ffi::OsString::from_wide(&buffer[..end]).into()))
}
/// A native file-drop clipboard payload, so paste transfers the HTML file itself.
pub fn copy_file(file: &Path) -> io::Result<()> {
    use windows_sys::Win32::{
        Foundation::GlobalFree,
        System::{DataExchange::*, Memory::*},
        UI::{Input::KeyboardAndMouse::GetActiveWindow, Shell::DROPFILES},
    };
    if !file.is_file() {
        return Err(io::Error::other("The shared file is missing."));
    }
    let mut path = wide(file.as_os_str());
    path.push(0);
    let size = std::mem::size_of::<DROPFILES>() + path.len() * 2;
    unsafe {
        let owner = GetActiveWindow();
        if owner.is_null() {
            return Err(io::Error::other("Activate Sidenote and try again."));
        }
        let memory = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, size);
        if memory.is_null() {
            return Err(io::Error::last_os_error());
        }
        let bytes = GlobalLock(memory);
        if bytes.is_null() {
            let error = io::Error::last_os_error();
            GlobalFree(memory);
            return Err(error);
        }
        let header = bytes.cast::<DROPFILES>();
        (*header).pFiles = std::mem::size_of::<DROPFILES>() as u32;
        (*header).fWide = 1;
        std::ptr::copy_nonoverlapping(
            path.as_ptr(),
            bytes
                .cast::<u8>()
                .add(std::mem::size_of::<DROPFILES>())
                .cast::<u16>(),
            path.len(),
        );
        GlobalUnlock(memory);
        if OpenClipboard(owner) == 0 {
            let error = io::Error::last_os_error();
            GlobalFree(memory);
            return Err(error);
        }
        // CF_HDROP is the Windows file-drop clipboard format (15).
        let result = if EmptyClipboard() == 0 || SetClipboardData(15, memory).is_null() {
            let error = io::Error::last_os_error();
            GlobalFree(memory);
            Err(error)
        } else {
            Ok(())
        };
        CloseClipboard();
        result
    }
}
