use std::os::unix::fs::MetadataExt;
use std::{
    ffi::{CStr, CString},
    fs, io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    },
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AgentUnavailable {
    UserNamespaceUnavailable = 1,
    CgroupNamespaceUnavailable,
    RootIsolationUnavailable,
    ProcessIsolationUnavailable,
    StagingSymlink,
    StagingHardlink,
    StagingSpecialFile,
    StagingTreeLimit,
    ExecSetupUnavailable,
    ChildDeathCheckUnavailable,
}
pub(super) fn decode(code: u8) -> AgentUnavailable {
    use AgentUnavailable::*;
    [
        UserNamespaceUnavailable,
        CgroupNamespaceUnavailable,
        RootIsolationUnavailable,
        ProcessIsolationUnavailable,
        StagingSymlink,
        StagingHardlink,
        StagingSpecialFile,
        StagingTreeLimit,
        ExecSetupUnavailable,
        ChildDeathCheckUnavailable,
    ]
    .get(usize::from(code).wrapping_sub(1))
    .copied()
    .unwrap_or(ProcessIsolationUnavailable)
}
pub(super) fn reason(e: &io::Error) -> AgentUnavailable {
    e.get_ref()
        .and_then(|e| e.downcast_ref().copied())
        .unwrap_or(AgentUnavailable::ProcessIsolationUnavailable)
}
impl std::fmt::Display for AgentUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AgentUnavailable: {self:?}")
    }
}
impl std::error::Error for AgentUnavailable {}
fn unshare(flags: i32, reason: AgentUnavailable) -> io::Result<()> {
    if unsafe { libc::unshare(flags) } == 0 {
        Ok(())
    } else {
        Err(io::Error::other(reason))
    }
}
pub(super) fn namespaces(agent: bool) -> io::Result<()> {
    use AgentUnavailable::*;
    let base = libc::CLONE_NEWUSER | libc::CLONE_NEWPID | libc::CLONE_NEWNET;
    if !agent {
        return unshare(base, ProcessIsolationUnavailable);
    }
    unshare(libc::CLONE_NEWUSER, UserNamespaceUnavailable)?;
    unshare(libc::CLONE_NEWCGROUP, CgroupNamespaceUnavailable)?;
    unshare(
        (base & !libc::CLONE_NEWUSER) | libc::CLONE_NEWNS | libc::CLONE_NEWIPC,
        ProcessIsolationUnavailable,
    )
}
fn check(ok: bool) -> Result<(), AgentUnavailable> {
    if ok {
        Ok(())
    } else {
        Err(AgentUnavailable::RootIsolationUnavailable)
    }
}
fn mount(dst: &CStr, flags: u64, options: Option<&CStr>) -> Result<(), AgentUnavailable> {
    let data = options.map_or(std::ptr::null(), |s| s.as_ptr().cast());
    check(
        unsafe {
            libc::mount(
                dst.as_ptr(),
                dst.as_ptr(),
                c"tmpfs".as_ptr(),
                flags as _,
                data,
            )
        } == 0,
    )
}
fn prctl(op: i32, arg: u64) -> bool {
    unsafe { libc::prctl(op, arg, 0, 0, 0) == 0 }
}
fn walk(dir: &fs::File, depth: usize, left: &mut usize) -> Result<(), AgentUnavailable> {
    use AgentUnavailable::*;
    if depth > 16 {
        return Err(StagingTreeLimit);
    }
    let entries = fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))
        .map_err(|_| RootIsolationUnavailable)?;
    for entry in entries {
        *left = left.checked_sub(1).ok_or(StagingTreeLimit)?;
        let name = CString::new(
            entry
                .map_err(|_| RootIsolationUnavailable)?
                .file_name()
                .as_bytes(),
        )
        .map_err(|_| StagingSpecialFile)?;
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(RootIsolationUnavailable);
        }
        let child = unsafe { fs::File::from_raw_fd(fd) };
        let m = child.metadata().map_err(|_| RootIsolationUnavailable)?;
        if m.file_type().is_symlink() {
            return Err(StagingSymlink);
        }
        if m.is_dir() {
            walk(&child, depth + 1, left)?;
        } else if !m.is_file() {
            return Err(StagingSpecialFile);
        } else if m.nlink() != 1 {
            return Err(StagingHardlink);
        }
    }
    Ok(())
}
pub(super) fn enter(root: fs::File) -> Result<(), AgentUnavailable> {
    unsafe {
        check(libc::getpid() == 1)?;
        let fail = |_| AgentUnavailable::RootIsolationUnavailable;
        let path = fs::read_link(format!("/proc/self/fd/{}", root.as_raw_fd())).map_err(fail)?;
        // Re-enter the copied mount tree: the inherited FD refers to the old namespace's mount.
        std::env::set_current_dir(&path).map_err(fail)?;
        let (a, b) = (
            fs::metadata(".").map_err(fail)?,
            root.metadata().map_err(fail)?,
        );
        check(a.dev() == b.dev() && a.ino() == b.ino())?;
        let names = ["work", "auth", ".old", "runtime", "meeting"];
        for name in names {
            check(fs::symlink_metadata(name).is_ok_and(|m| m.is_dir()))?;
        }
        for entry in fs::read_dir(".").map_err(|_| AgentUnavailable::RootIsolationUnavailable)? {
            check(entry.is_ok_and(|e| names.iter().any(|n| e.file_name() == *n)))?;
        }
        walk(&root, 0, &mut 512)?;
        // Non-recursive bind excludes host submounts; only trusted staged copies are admitted.
        mount(c"/", libc::MS_PRIVATE | libc::MS_REC, None)?;
        mount(c".", libc::MS_BIND, None)?;
        std::env::set_current_dir(&path).map_err(fail)?;
        let safe = libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC;
        mount(c"work", safe, Some(c"size=8388608,nr_inodes=128,mode=700"))?;
        mount(c"auth", safe, Some(c"size=1048576,nr_inodes=32,mode=700"))?;
        let ro =
            libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV;
        mount(c".", ro, None)?;
        let mut stat = std::mem::zeroed::<libc::statvfs>();
        check(libc::statvfs(c".".as_ptr(), &mut stat) == 0 && stat.f_flag & libc::ST_RDONLY != 0)?;
        check(libc::syscall(libc::SYS_pivot_root, c".".as_ptr(), c".old".as_ptr()) == 0)?;
        check(
            libc::chdir(c"/".as_ptr()) == 0
                && libc::umount2(c"/.old".as_ptr(), libc::MNT_DETACH) == 0,
        )?;
        drop(root);
        check(
            prctl(libc::PR_SET_DUMPABLE, 0)
                && prctl(libc::PR_CAP_AMBIENT, libc::PR_CAP_AMBIENT_CLEAR_ALL as _),
        )?;
        for cap in 0..64 {
            check(
                prctl(libc::PR_CAPBSET_DROP, cap)
                    || io::Error::last_os_error().raw_os_error() == Some(libc::EINVAL),
            )?;
        }
        let header = [0x20080522u32, 0];
        let mut data = [0u32; 6];
        check(libc::syscall(libc::SYS_capset, header.as_ptr(), data.as_ptr()) == 0)?;
        check(prctl(libc::PR_SET_NO_NEW_PRIVS, 1))?;
        check(
            libc::syscall(libc::SYS_capget, header.as_ptr(), data.as_mut_ptr()) == 0
                && data == [0; 6],
        )?;
        check(libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) == 0)?;
    }
    Ok(())
}
