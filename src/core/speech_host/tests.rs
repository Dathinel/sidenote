use super::*;
const TOKEN: &[u8] = b"synthetic-speech-host-test-value";

fn contains_token(bytes: &[u8]) -> bool {
    bytes.windows(TOKEN.len()).any(|window| window == TOKEN)
}

fn consented(path: &Path) -> Host {
    Host::save_and_consent(path, "https://host.example", TOKEN).unwrap()
}

#[test]
fn rejects_http_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.dpapi");
    let result = Host::save_and_consent(&path, "http://localhost", TOKEN);
    assert_eq!(
        result.err().as_deref(),
        Some("The speech host URL must use HTTPS.")
    );
    assert!(!path.exists());
}

#[test]
fn token_stays_out_of_written_config() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.json");
    store_io::write(&config, &serde_json::json!({"version": 1})).unwrap();
    let before = fs::read(&config).unwrap();
    let path = dir.path().join("host.dpapi");
    consented(&path);
    let after = fs::read(&config).unwrap();
    assert!(!contains_token(&after), "config contained a token");
    assert!(before == after, "speech host changed config.json");
    assert!(
        !contains_token(&fs::read(&path).unwrap()),
        "protected file contained plaintext"
    );
    let loaded = Host::load(&path).unwrap().unwrap();
    assert_eq!(loaded.recipient(), "https://host.example");
    assert!(loaded.token.as_slice() == TOKEN);
}
