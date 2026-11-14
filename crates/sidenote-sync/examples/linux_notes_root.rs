//! Synthetic, owned-unit proof only; never wired to the worker CLI or Ready gate.
#[cfg(target_os = "linux")]
fn main() {
    use sidenote_sync::{notes_cgroup::Budget, pid_worker::agent_notes};
    use std::{fs, os::unix::process::CommandExt, process::Command};
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if let Some(root) = args.first() {
        let pinned = fs::File::open(root).unwrap();
        if std::env::var("SIDENOTE_ROOT_PREEXEC").as_deref() == Ok("1") {
            let limit = libc::rlimit {
                rlim_cur: 268435456,
                rlim_max: 268435456,
            };
            assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_AS, &limit) }, 0);
        }
        let program = if args.len() == 1 {
            &["/runtime/probe".into()][..]
        } else {
            &args[1..]
        };
        let result = agent_notes(program, pinned);
        if let Ok(expected) = std::env::var("SIDENOTE_ROOT_EXPECT") {
            assert_eq!(format!("{:?}", result.unwrap_err()), expected);
            println!("DENY typed {expected}");
        } else if args.len() == 1 {
            assert_eq!(
                result,
                Err(sidenote_sync::pid_worker::AgentUnavailable::RootIsolationUnavailable)
            );
        } else {
            assert_eq!(result, Ok(0));
        }
        return;
    }
    let budget = Budget::prepare(&std::env::var("SIDENOTE_TEST_UNIT").unwrap()).unwrap();
    let leaf = budget.job(2).unwrap();
    let parent = leaf.0.parent().unwrap();
    let supervisor = parent.join("supervisor");
    let targets: Vec<_> = [leaf.0.as_path(), parent, &supervisor]
        .into_iter()
        .flat_map(|p| [p.join("memory.max"), p.join("cgroup.procs")])
        .collect();
    let temp = tempfile::tempdir().unwrap();
    let stage = temp.path().join("root");
    for name in ["runtime", "meeting", "work", "auth", ".old"] {
        fs::create_dir_all(stage.join(name)).unwrap();
    }
    fs::write(stage.join("meeting/input"), "synthetic").unwrap();
    let outside = temp.path().join("outside");
    fs::write(&outside, "canary").unwrap();
    let source = temp.path().join("probe.c");
    fs::write(&source, PROBE).unwrap();
    let probe = stage.join("runtime/probe");
    let status = Command::new("/usr/bin/gcc")
        .arg("-static")
        .arg(&source)
        .arg("-o")
        .arg(&probe)
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new(&probe)
        .arg("host")
        .arg(&outside)
        .args(&targets)
        .status()
        .unwrap();
    assert!(status.success());
    let trial = |reason: &str, program: &str, preexec: bool| {
        let status = Command::new(std::env::current_exe().unwrap())
            .arg(&stage)
            .arg(program)
            .env("SIDENOTE_ROOT_EXPECT", reason)
            .env("SIDENOTE_ROOT_PREEXEC", if preexec { "1" } else { "0" })
            .status()
            .unwrap();
        assert!(status.success());
    };
    let bad = stage.join("meeting/bad");
    for kind in ["symlink", "hardlink", "fifo", "deep", "large"] {
        let reason = match kind {
            "symlink" => {
                std::os::unix::fs::symlink(&outside, &bad).unwrap();
                assert_eq!(fs::read_to_string(&bad).unwrap(), "canary");
                "StagingSymlink"
            }
            "hardlink" => {
                fs::hard_link(&outside, &bad).unwrap();
                assert_eq!(fs::read_to_string(&bad).unwrap(), "canary");
                "StagingHardlink"
            }
            "fifo" => {
                let path = std::ffi::CString::new(bad.as_os_str().as_encoded_bytes()).unwrap();
                assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
                "StagingSpecialFile"
            }
            _ => {
                let mut path = bad.clone();
                fs::create_dir(&path).unwrap();
                if kind == "deep" {
                    for _ in 0..17 {
                        path = path.join("d");
                        fs::create_dir(&path).unwrap();
                    }
                } else {
                    for n in 0..513 {
                        fs::write(path.join(n.to_string()), "").unwrap();
                    }
                }
                assert!(path.is_dir());
                "StagingTreeLimit"
            }
        };
        println!("CONTROL existing {kind} staging fixture");
        trial(reason, "/runtime/probe", false);
        if bad.is_dir() {
            fs::remove_dir_all(&bad).unwrap();
        } else {
            fs::remove_file(&bad).unwrap();
        }
    }
    trial("ExecSetupUnavailable", "/runtime/missing", false);
    trial("ExecSetupUnavailable", "/runtime/probe", true);
    let mut command = Command::new(std::env::current_exe().unwrap());
    assert!(command.arg(temp.path()).status().unwrap().success());
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .arg(&stage)
        .arg("/runtime/probe")
        .arg(&outside)
        .args(&targets);
    let placement = leaf.0.join("cgroup.procs");
    unsafe {
        command.pre_exec(move || fs::write(&placement, "0"));
    }
    assert!(command.status().unwrap().success());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "canary");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(leaf.remove()).unwrap();
    assert!(!leaf.0.exists());
    println!("PASS canary unchanged; job leaf removed; temporary staging removed on return");
}
#[cfg(not(target_os = "linux"))]
fn main() {}
#[cfg(target_os = "linux")]
const PROBE: &str = r#"
#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <sys/statvfs.h>
#include <unistd.h>
int main(int argc,char **argv) {
 int host=argc==9 && !strcmp(argv[1],"host"), offset=host?2:1; assert(argc==offset+7);
 char *outside=argv[offset], renamed[4096]; snprintf(renamed,sizeof renamed,"%s.new",outside);
 int fd=open(outside,O_RDONLY); if(host){assert(fd>=0); close(fd);}else assert(fd<0 && errno==ENOENT);
 fd=open(renamed,O_CREAT|O_WRONLY,0600); if(host){assert(fd>=0); close(fd); assert(!unlink(renamed));}else assert(fd<0 && errno==ENOENT);
 int r=chmod(outside,0600); if(host)assert(!r);else assert(r<0 && errno==ENOENT);
 r=rename(outside,renamed); if(host){assert(!r); assert(!rename(renamed,outside));}else assert(r<0 && errno==ENOENT);
 puts(host?"CONTROL outside read/create/chmod/rename reachable":"DENY outside read/create/chmod/rename");
 for(int i=offset+1;i<argc;i++) {
  fd=open(argv[i],O_RDWR); if(host){assert(fd>=0); if(strstr(argv[i],"memory.max")){char old[64]; int n=read(fd,old,sizeof old); assert(n>0 && lseek(fd,0,SEEK_SET)==0 && write(fd,old,n)==n);}else{errno=0; assert(write(fd,"2147483647",10)<0); assert(errno==ESRCH || errno==EBUSY);} close(fd);}else assert(fd<0 && errno==ENOENT);
  printf("%s %s\n",host?"CONTROL cgroup file reachable":"DENY cgroup file absent",argv[i]);
 }
 if(host){char path[4096]; snprintf(path,sizeof path,"%s.auth-control",outside); fd=open(path,O_CREAT|O_RDWR,0600); assert(fd>=0); char b[4096]={0}; for(int i=0;i<512;i++)assert(write(fd,b,sizeof b)==sizeof b); close(fd); assert(!unlink(path)); puts("CONTROL 2MiB write succeeds outside job"); return 0;}
 struct statvfs s; assert(!statvfs("/",&s) && (s.f_flag&ST_RDONLY));
 assert(!statvfs("/auth",&s) && s.f_blocks*s.f_frsize<=1048576);
 assert(access("/proc",F_OK)<0 && access("/sys",F_OK)<0 && access("/.old/proc",F_OK)<0);
 assert(getppid()==1 && prctl(PR_GET_NO_NEW_PRIVS,0,0,0,0)==1);
 fd=open("/meeting/input",O_WRONLY); assert(fd<0 && errno==EROFS);
 assert(chmod("/meeting/input",0600)<0 && errno==EROFS);
 assert(rename("/meeting/input","/meeting/moved")<0 && errno==EROFS);
 fd=open("/auth/fill",O_CREAT|O_WRONLY,0600); assert(fd>=0); char b[4096]={0}; long total=0,n; while((n=write(fd,b,sizeof b))>0)total+=n; assert(errno==ENOSPC && total<=1048576); close(fd);
 fd=open("/work/ok",O_CREAT|O_WRONLY,0600); assert(fd>=0 && write(fd,"ok",2)==2); close(fd);
 puts("DENY auth ENOSPC at 1MiB; PASS work writable, root/meeting read-only, no proc/sys/old root, PID1 parent and no_new_privs"); puts("FD_READY"); fflush(stdout); sleep(2); return 0;
}
"#;
