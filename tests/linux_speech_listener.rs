#![cfg(all(target_os = "linux", feature = "nightly"))]
#[path = "../src/service/speech/listener_tests.rs"]
mod tests;
