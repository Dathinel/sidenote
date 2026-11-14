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
    CgroupBudgetUnavailable,
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
    Ready(u128, u128),
    Material(u128, Option<u128>, String),
}
struct Length(usize);
impl std::io::Write for Length {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let next = self.0.checked_add(bytes.len()).filter(|&n| n <= LIMIT);
        self.0 = next.ok_or(std::io::ErrorKind::InvalidData)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encoded_len(frame: &Frame) -> Result<usize, Failure> {
    let mut length = Length(1);
    serde_json::to_writer(&mut length, frame).map_err(|_| Failure::Oversized)?;
    Ok(length.0)
}
fn encode(frame: &Frame) -> Result<Vec<u8>, Failure> {
    let length = encoded_len(frame)?;
    let mut bytes = Vec::with_capacity(length);
    bytes.push(1);
    serde_json::to_writer(&mut bytes, frame).map_err(|_| Failure::InvalidFrame)?;
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
        let mut material = Frame::Material(job.0, Some(u128::MAX), text);
        encoded_len(&material)?;
        self.write(&Frame::Begin(job)).await?;
        let token = match self.read().await? {
            Frame::Ready(nonce, token) if nonce == job.0 => token,
            _ => return Err(Failure::BeforeReady),
        };
        if let Frame::Material(_, proof, _) = &mut material {
            *proof = Some(token);
        }
        self.write(&material).await
    }
}
#[derive(Default)]
pub struct Supervisor {
    seen: BTreeSet<u128>,
    busy: bool,
    owned: Option<Child>,
    budget: Option<super::notes_cgroup::Budget>,
    leaf: Option<super::notes_cgroup::Leaf>,
}
impl Supervisor {
    pub fn owned_pid(&self) -> Option<u32> {
        self.owned.as_ref().and_then(Child::id)
    }
    pub async fn finish(&mut self) -> Result<(), Failure> {
        self.busy = false;
        if let Some(leaf) = &self.leaf {
            leaf.kill()?;
        }
        if let Some(mut child) = self.owned.take() {
            let killed = child.start_kill();
            killed.and(child.wait().await.map(|_| ())).map_err(io)?;
        }
        if let Some(leaf) = &self.leaf {
            leaf.remove()?;
            self.leaf = None;
        }
        Ok(())
    }
    pub async fn run(
        &mut self,
        pipes: &mut Pipes,
        unit: &str,
        setup: impl FnOnce(Stage, Model, &std::path::Path) -> Result<Option<Child>, Failure>,
    ) -> Result<String, Failure> {
        if self.budget.is_none() {
            self.budget = Some(super::notes_cgroup::Budget::prepare(unit)?);
        }
        self.exchange(pipes, setup).await
    }
    async fn exchange(
        &mut self,
        pipes: &mut Pipes,
        setup: impl FnOnce(Stage, Model, &std::path::Path) -> Result<Option<Child>, Failure>,
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
            if let Some(budget) = &self.budget {
                self.leaf = Some(budget.job(nonce)?);
            }
            self.owned = setup(
                stage,
                model,
                self.leaf
                    .as_ref()
                    .map_or(std::path::Path::new(""), |l| &l.0),
            )?;
            if let (Some(leaf), Some(pid)) = (&self.leaf, self.owned_pid()) {
                leaf.contains(pid)?;
            }
            let mut token = 0u128;
            let pointer = std::ptr::addr_of_mut!(token).cast();
            if unsafe { libc::getrandom(pointer, 16, libc::GRND_NONBLOCK) } != 16 {
                return Err(Failure::Io);
            }
            pipes.write(&Frame::Ready(nonce, token)).await?;
            match pipes.read().await? {
                Frame::Material(n, Some(proof), text) if n == nonce && proof == token => Ok(text),
                Frame::Material(..) => Err(Failure::BeforeReady),
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
        let deny = |_, _, _: &std::path::Path| panic!("no launch");
        let stub = |_, _, _: &std::path::Path| Ok(None);
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
                assert!(s.exchange(&mut host, deny).await.is_err());
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
                s.exchange(&mut host, stub),
                server.submit((1, stage, Model::ClaudeSonnet55), "ok".into())
            );
            assert_eq!(result, Ok("ok".into()));
            sent.unwrap();
            for reason in [Failure::Busy, Failure::Replay] {
                server.write(&begin).await.unwrap();
                assert_eq!(s.exchange(&mut host, stub).await, Err(reason));
            }
            s.seen = (0..1024).collect();
            let next = Frame::Begin((1024, stage, Model::ClaudeSonnet55));
            server.write(&next).await.unwrap();
            assert_eq!(s.exchange(&mut host, deny).await, Err(Failure::Busy));
            assert_eq!(s.seen, (0..1024).collect());
        }
        let mut stale = None;
        for phase in 0usize..5 {
            let (mut host, mut server) = pair();
            let mut s = Supervisor::default();
            let client = async {
                if phase > 0 {
                    server.write(&begin).await.unwrap();
                }
                if phase > 1 {
                    let Frame::Ready(_, token) = server.read().await.unwrap() else {
                        panic!("Ready");
                    };
                    stale = Some(token);
                    if phase > 2 {
                        let frame = if phase == 4 {
                            begin.clone()
                        } else {
                            Frame::Material(1, Some(token), "ok".into())
                        };
                        server.write(&frame).await.unwrap();
                    }
                }
                drop(server);
            };
            let (result, _) = tokio::join!(s.exchange(&mut host, stub), client);
            assert_eq!(result.is_ok(), phase == 3);
            assert_eq!(s.exchange(&mut host, stub).await, Err(Failure::Eof));
        }
        for proof in [None, stale] {
            let (mut host, mut server) = pair();
            let mut s = Supervisor::default();
            let mut command = tokio::process::Command::new("/bin/sleep");
            let child = command.arg("30").spawn().unwrap();
            let pid = child.id().unwrap();
            let material = Frame::Material(1, proof, "early or stale".into());
            server.write(&begin).await.unwrap();
            if proof.is_none() {
                server.write(&material).await.unwrap();
            }
            let client = async {
                let Frame::Ready(_, token) = server.read().await.unwrap() else {
                    panic!("Ready");
                };
                assert!(Some(token) != stale);
                if proof.is_some() {
                    server.write(&material).await.unwrap();
                }
            };
            let (result, _) =
                tokio::join!(s.exchange(&mut host, |_, _, _| Ok(Some(child))), client);
            assert_eq!(result, Err(Failure::BeforeReady));
            assert_eq!(s.owned_pid(), None);
            let reaped = unsafe { libc::waitpid(pid as i32, std::ptr::null_mut(), 1) };
            assert_eq!(reaped, -1);
        }
        let mut s = Supervisor::default();
        let (mut host, mut server) = pair();
        server.tx.write_u32((LIMIT + 1) as u32).await.unwrap();
        assert_eq!(s.exchange(&mut host, deny).await, Err(Failure::Oversized));
        assert_eq!(host.read().await, Err(Failure::Deadline));
        let material = |n| Frame::Material(1, Some(1), "x".repeat(n));
        let overhead = encoded_len(&material(0)).unwrap();
        let exact = material(LIMIT - overhead);
        assert_eq!(encode(&exact).unwrap().len(), LIMIT);
        assert!(std::io::Write::write(&mut Length(usize::MAX), b"x").is_err());
        let oversized = material(LIMIT - overhead + 1);
        assert_eq!(encoded_len(&oversized), Err(Failure::Oversized));
        assert_eq!(encode(&oversized), Err(Failure::Oversized));
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
