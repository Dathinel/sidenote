use std::{io, os::windows::ffi::OsStrExt, path::Path};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow},
};
fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}
pub fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    move_file(from, to, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
}
/// Durable publication of an immutable file. An existing destination is never replaced.
pub fn publish_file(from: &Path, to: &Path) -> io::Result<()> {
    move_file(from, to, MOVEFILE_WRITE_THROUGH)
}
fn move_file(from: &Path, to: &Path, flags: u32) -> io::Result<()> {
    if unsafe {
        MoveFileExW(
            wide(from.as_os_str()).as_ptr(),
            wide(to.as_os_str()).as_ptr(),
            flags,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
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
            let title = wide(std::ffi::OsStr::new("Meetings"));
            let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
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
