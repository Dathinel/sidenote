#[cfg(target_os = "linux")]
#[path = "user_runner/fixture.rs"]
mod fixture;
fn main() {
    #[cfg(target_os = "linux")]
    {
        sidenote::platform::supervisor_entry();
        fixture::run();
    }
}
