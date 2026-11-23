use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
use windows_sys::Win32::{
    Foundation::WAIT_OBJECT_0,
    System::Threading::{
        CreateEventW, EVENT_MODIFY_STATE, OpenEventW, SYNCHRONIZATION_SYNCHRONIZE, SetEvent,
        WaitForMultipleObjects, WaitForSingleObject,
    },
};

pub(super) struct Signal(OwnedHandle);
impl Signal {
    pub(super) fn attach(command: &mut Command, label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "Local\\sidenote-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        command.args(["--skip", &format!("{label}={name}")]);
        let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        let raw = unsafe { CreateEventW(std::ptr::null(), 1, 0, wide.as_ptr()) };
        assert!(!raw.is_null());
        Self(unsafe { OwnedHandle::from_raw_handle(raw) })
    }
    pub(super) fn wait(&self, process: &OwnedHandle) {
        let handles = [self.0.as_raw_handle(), process.as_raw_handle()];
        assert_eq!(
            unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, 120_000) },
            WAIT_OBJECT_0,
            "fixture exited or did not become ready within its startup guard"
        );
    }
    pub(super) fn wait_active(&self) {
        assert_eq!(
            unsafe { WaitForSingleObject(self.0.as_raw_handle(), 120_000) },
            WAIT_OBJECT_0,
            "fixture did not receive the request within its setup guard"
        );
    }
    pub(super) fn notify(label: &str) {
        if let Some(signal) = Self::from_args(label, EVENT_MODIFY_STATE) {
            signal.set();
        }
    }
    pub(super) fn open_for_wait(label: &str) -> Self {
        Self::from_args(label, SYNCHRONIZATION_SYNCHRONIZE).expect("missing release signal")
    }
    pub(super) fn set(&self) {
        assert_ne!(unsafe { SetEvent(self.0.as_raw_handle()) }, 0);
    }
    fn from_args(label: &str, access: u32) -> Option<Self> {
        let prefix = format!("{label}=");
        let name = std::env::args().find_map(|a| a.strip_prefix(&prefix).map(str::to_owned))?;
        let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        let raw = unsafe { OpenEventW(access, 0, wide.as_ptr()) };
        assert!(!raw.is_null());
        Some(Self(unsafe { OwnedHandle::from_raw_handle(raw) }))
    }
}
