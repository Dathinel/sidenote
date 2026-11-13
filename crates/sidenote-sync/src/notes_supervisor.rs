use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, os::fd::BorrowedFd, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::unix::pipe::{Receiver, Sender},
    process::Child,
    time::timeout,
};
const LIMIT: usize = 65536;
const DEADLINE: Duration = Duration::from_secs(5);
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    InvalidFrame,
    Oversized,
    Eof,
    Deadline,
    Io,
    Busy,
    Replay,
    BeforeReady,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    Title,
    Notes,
    Tasks,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Model {
    ClaudeSonnet55,
}
pub type Job = (u128, Stage, Model);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Frame {
    Begin(Job),
    Ready(u128),
    Material(u128, String),
}
fn encode(frame: &Frame) -> Result<Vec<u8>, Failure> {
    let mut bytes = vec![1];
    bytes.extend(serde_json::to_vec(frame).map_err(|_| Failure::InvalidFrame)?);
    if bytes.len() > LIMIT {
        return Err(Failure::Oversized);
    }
    Ok(bytes)
}
fn io(e: std::io::Error) -> Failure {
    match e.kind() {
        std::io::ErrorKind::UnexpectedEof => Failure::Eof,
        _ => Failure::Io,
    }
}
pub struct Pipes {
    rx: Receiver,
    tx: Sender,
}
impl Pipes {
    pub fn inherited() -> Result<Self, Failure> {
        let fd = |n| unsafe {
            let mut stat = std::mem::zeroed::<libc::statfs>();
            if libc::fstatfs(n, &mut stat) != 0
                || stat.f_type != 0x50495045
                || libc::fcntl(n, libc::F_SETFD, libc::FD_CLOEXEC) < 0
            {
                return Err(Failure::Io);
            }
            BorrowedFd::borrow_raw(n).try_clone_to_owned().map_err(io)
        };
        Ok(Self {
            rx: Receiver::from_owned_fd(fd(0)?).map_err(io)?,
            tx: Sender::from_owned_fd(fd(1)?).map_err(io)?,
        })
    }
    pub async fn read(&mut self) -> Result<Frame, Failure> {
        timeout(DEADLINE, async {
            let length = self.rx.read_u32().await.map_err(io)? as usize;
            if length > LIMIT {
                return Err(Failure::Oversized);
            }
            let mut bytes = vec![0; length];
            self.rx.read_exact(&mut bytes).await.map_err(io)?;
            if bytes.first() != Some(&1) {
                return Err(Failure::InvalidFrame);
            }
            serde_json::from_slice(&bytes[1..]).map_err(|_| Failure::InvalidFrame)
        })
        .await
        .map_err(|_| Failure::Deadline)?
    }
    pub async fn write(&mut self, frame: &Frame) -> Result<(), Failure> {
        let bytes = encode(frame)?;
        timeout(DEADLINE, async {
            self.tx.write_u32(bytes.len() as u32).await.map_err(io)?;
            self.tx.write_all(&bytes).await.map_err(io)
        })
        .await
        .map_err(|_| Failure::Deadline)?
    }
    pub async fn submit(&mut self, job: Job, text: String) -> Result<(), Failure> {
        let material = Frame::Material(job.0, text);
        encode(&material)?;
        self.write(&Frame::Begin(job)).await?;
        if self.read().await? != Frame::Ready(job.0) {
            return Err(Failure::BeforeReady);
        }
        self.write(&material).await
    }
}
#[derive(Default)]
pub struct Supervisor {
    seen: BTreeSet<u128>,
    busy: bool,
    owned: Option<Child>,
}
impl Supervisor {
    pub fn owned_pid(&self) -> Option<u32> {
        self.owned.as_ref().and_then(Child::id)
    }
    pub async fn finish(&mut self) -> Result<(), Failure> {
        self.busy = false;
        if let Some(mut child) = self.owned.take() {
            let killed = child.start_kill();
            killed.and(child.wait().await.map(|_| ())).map_err(io)?;
        }
        Ok(())
    }
    pub async fn run(
        &mut self,
        pipes: &mut Pipes,
        setup: impl FnOnce(Stage, Model) -> Result<Option<Child>, Failure>,
    ) -> Result<String, Failure> {
        let result = async {
            let Frame::Begin((nonce, stage, model)) = pipes.read().await? else {
                return Err(Failure::BeforeReady);
            };
            if self.busy || self.seen.len() == 1024 {
                return Err(Failure::Busy);
            }
            if self.seen.contains(&nonce) {
                return Err(Failure::Replay);
            }
            self.seen.insert(nonce);
            self.busy = true;
            use std::os::fd::AsRawFd;
            let check = || {
                let mut pending = 0;
                if unsafe { libc::ioctl(pipes.rx.as_raw_fd(), libc::FIONREAD, &mut pending) } != 0 {
                    return Err(Failure::Io);
                }
                if pending != 0 {
                    return Err(Failure::BeforeReady);
                }
                Ok(())
            };
            check()?;
            self.owned = setup(stage, model)?;
            check()?;
            pipes.write(&Frame::Ready(nonce)).await?;
            match pipes.read().await? {
                Frame::Material(n, text) if n == nonce => Ok(text),
                Frame::Begin(..) => Err(Failure::Busy),
                _ => Err(Failure::InvalidFrame),
            }
        }
        .await;
        if result.is_err() {
            self.finish().await?;
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn pair() -> (Pipes, Pipes) {
        let (tx, rx) = tokio::net::unix::pipe::pipe().unwrap();
        let (tx2, rx2) = tokio::net::unix::pipe::pipe().unwrap();
        (Pipes { rx, tx: tx2 }, Pipes { rx: rx2, tx })
    }
    #[tokio::test(start_paused = true)]
    async fn contract() {
        let deny = |_, _| panic!("no launch");
        let stub = |_, _| Ok(None);
        let begin = Frame::Begin((1, Stage::Title, Model::ClaudeSonnet55));
        let mut bytes = encode(&begin).unwrap();
        let mut bad = vec![vec![], vec![2], vec![1], vec![1, b'{']];
        for old in ["Title", "Claude", "Begin", "[1,"] {
            let text = String::from_utf8(bytes.clone()).unwrap();
            bad.push(text.replace(old, "-1,").into_bytes());
        }
        for wire in &mut bad {
            wire.splice(0..0, (wire.len() as u32).to_be_bytes());
        }
        bytes.splice(0..0, (bytes.len() as u32).to_be_bytes());
        bad.extend((0..bytes.len()).map(|n| bytes[..n].to_vec()));
        for wire in bad {
            for owned in [false, true] {
                let (mut host, mut server) = pair();
                let mut s = Supervisor::default();
                if owned {
                    let mut command = tokio::process::Command::new("/bin/sleep");
                    s.owned = Some(command.arg("30").spawn().unwrap());
                }
                let pid = s.owned_pid();
                server.tx.write_all(&wire).await.unwrap();
                drop(server);
                assert!(s.run(&mut host, deny).await.is_err());
                assert_eq!(s.owned_pid(), None);
                if let Some(pid) = pid {
                    let reaped = unsafe { libc::waitpid(pid as i32, std::ptr::null_mut(), 1) };
                    assert_eq!(reaped, -1);
                }
            }
        }
        for stage in [Stage::Title, Stage::Notes, Stage::Tasks] {
            let (mut host, mut server) = pair();
            let mut s = Supervisor::default();
            let (result, sent) = tokio::join!(
                s.run(&mut host, stub),
                server.submit((1, stage, Model::ClaudeSonnet55), "ok".into())
            );
            assert_eq!(result, Ok("ok".into()));
            sent.unwrap();
            for reason in [Failure::Busy, Failure::Replay] {
                server.write(&begin).await.unwrap();
                assert_eq!(s.run(&mut host, stub).await, Err(reason));
            }
        }
        for phase in 0usize..5 {
            let (mut host, mut server) = pair();
            let mut s = Supervisor::default();
            let client = async {
                let frames = [begin.clone(), Frame::Material(1, "ok".into())];
                for (index, frame) in frames.iter().take(phase.div_ceil(2)).enumerate() {
                    let frame = if phase == 4 { &begin } else { frame };
                    server.write(frame).await.unwrap();
                    if index == 0 && phase > 1 {
                        server.read().await.unwrap();
                    }
                }
                drop(server);
            };
            let (result, _) = tokio::join!(s.run(&mut host, stub), client);
            assert_eq!(result.is_ok(), phase == 3);
            assert_eq!(s.run(&mut host, stub).await, Err(Failure::Eof));
        }
        let (mut host, mut server) = pair();
        let mut s = Supervisor::default();
        server.write(&begin).await.unwrap();
        let mut early = encode(&Frame::Material(1, "early".into())).unwrap();
        early.splice(0..0, (early.len() as u32).to_be_bytes());
        let setup = |_, _| server.tx.try_write(&early).map(|_| None).map_err(io);
        assert_eq!(s.run(&mut host, setup).await, Err(Failure::BeforeReady));
        let (mut host, mut server) = pair();
        server.tx.write_u32((LIMIT + 1) as u32).await.unwrap();
        assert_eq!(s.run(&mut host, deny).await, Err(Failure::Oversized));
        assert_eq!(host.read().await, Err(Failure::Deadline));
        let overhead = encode(&Frame::Material(1, String::new())).unwrap().len();
        let material = |n| Frame::Material(1, "x".repeat(n));
        let exact = material(LIMIT - overhead);
        assert_eq!(encode(&exact).unwrap().len(), LIMIT);
        let over = encode(&material(LIMIT - overhead + 1));
        assert_eq!(over, Err(Failure::Oversized));
        let (read, sent) = tokio::join!(host.read(), server.write(&exact));
        assert_eq!((read, sent), (Ok(exact.clone()), Ok(())));
        assert_eq!(server.write(&exact).await, Err(Failure::Deadline));
        let (mut host, mut server) = pair();
        let send = async {
            for byte in bytes {
                server.tx.write_all(&[byte]).await.unwrap();
                tokio::task::yield_now().await;
            }
        };
        let (result, _) = tokio::join!(host.read(), send);
        assert_eq!(result, Ok(begin));
    }
}
