// Production entries require reviewed public request/tool/isolation/retention and route proofs.
// A helper self-report never grants qualification; the bundled production catalog is empty.
use super::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Read;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    executable: String,
    model: String,
    version: String,
    provider: String,
    auth_route: String,
    invocations: Vec<String>,
}
pub(in crate::core::runner_process) struct Qualified {
    pub version: String,
    pub provider: String,
    pub auth_route: String,
    _pin: fs::File,
}
pub(in crate::core::runner_process) fn refusal() -> RunnerFailure {
    RunnerFailure {
        category: "HELPER_UNQUALIFIED",
        exit: None,
        message: "Translation provider has not been qualified.".into(),
    }
}
fn hash(file: &mut fs::File) -> std::io::Result<String> {
    let mut digest = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        digest.update(&bytes[..n]);
    }
    Ok(hex::encode(digest.finalize()))
}
fn fingerprint(runner: &Runner) -> String {
    hex::encode(Sha256::digest(
        serde_json::to_vec(&(&runner.args, super::configuration())).unwrap(),
    ))
}
pub(in crate::core::runner_process) fn admit(
    program: &Path,
    runner: &Runner,
) -> Result<Qualified, RunnerFailure> {
    let records: Vec<Receipt> =
        serde_json::from_str(include_str!("qualified.json")).map_err(|_| refusal())?;
    // Test trust anchor is only the native fixture binary, never an installed provider.
    #[cfg(test)]
    let records = records
        .into_iter()
        .chain(fixture_receipts())
        .collect::<Vec<_>>();
    if records.is_empty() {
        return Err(refusal());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut pin = options.open(program).map_err(|_| refusal())?;
    let executable = hash(&mut pin).map_err(|_| refusal())?;
    let invocation = fingerprint(runner);
    let record = records
        .into_iter()
        .find(|r| {
            !r.version.is_empty()
                && r.executable == executable
                && Some(r.model.as_str()) == runner.model.as_deref()
                && r.invocations.contains(&invocation)
        })
        .ok_or_else(refusal)?;
    Ok(Qualified {
        version: record.version,
        provider: record.provider,
        auth_route: record.auth_route,
        _pin: pin,
    })
}
#[cfg(test)]
fn fixture_receipts() -> Vec<Receipt> {
    static RECORDS: std::sync::OnceLock<Vec<Receipt>> = std::sync::OnceLock::new();
    RECORDS
        .get_or_init(|| {
            let fixture =
                hash(&mut fs::File::open(std::env::current_exe().unwrap()).unwrap()).unwrap();
            [
                "translation-fixture",
                "translation-original",
                "translation-stale-binary",
                "translation-stale-invocation",
            ]
            .into_iter()
            .map(|model| {
                let mut runner = super::runner(model).unwrap();
                let normal = fingerprint(&runner);
                runner.args.push("--sidenote-capability".into());
                Receipt {
                    executable: if model == "translation-stale-binary" {
                        "0".repeat(64)
                    } else {
                        fixture.clone()
                    },
                    model: model.into(),
                    version: "native-fixture-v1".into(),
                    provider: "fixture".into(),
                    auth_route: "subscription".into(),
                    invocations: if model == "translation-stale-invocation" {
                        vec!["0".repeat(64)]
                    } else {
                        vec![normal, fingerprint(&runner)]
                    },
                }
            })
            .collect()
        })
        .clone()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::core::runner_process) struct Capability {
    version: String,
    model: String,
    tools: Vec<String>,
    isolated: bool,
    retains_content: bool,
    pub route: Option<Route>,
}
pub(in crate::core::runner_process) fn capability(
    text: &str,
    model: &str,
    qualified: &Qualified,
) -> Result<Capability, RunnerFailure> {
    if text.len() > 8192 {
        return Err(refusal());
    }
    let cap: Capability = serde_json::from_str(text).map_err(|_| refusal())?;
    if cap.version != qualified.version
        || cap.model != model
        || !cap.tools.is_empty()
        || !cap.isolated
        || cap.retains_content
    {
        return Err(refusal());
    }
    Ok(cap)
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::core::runner_process) struct Route {
    pub provider: String,
    pub id: String,
    pub auth_mode: String,
    pub location: String,
    pub payment: String,
    pub automatic_fallback: bool,
}
pub(in crate::core::runner_process) fn payment_allowed(
    cap: &Capability,
    qualified: &Qualified,
    location: &str,
    payment: &str,
) -> bool {
    cap.route.as_ref().is_some_and(|route| {
        route.provider == qualified.provider
            && route.id == qualified.auth_route
            && route.auth_mode == "subscription"
            && route.location == location
            && route.payment == payment
            && payment == "subscription"
            && !route.automatic_fallback
    })
}
pub(in crate::core::runner_process) fn payment_refusal() -> RunnerFailure {
    RunnerFailure {
        category: "HELPER_PAYMENT",
        exit: None,
        message: "Translation subscription route is unavailable.".into(),
    }
}
