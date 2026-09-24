//! On-ring pairing, shared by the desktop CLI and the iOS FFI.
//!
//! A ring accepts a new auth key only while it is factory-reset. [`pair`] checks
//! that state first, installs the caller's key, verifies it, aligns the ring
//! clock, reads the battery, and turns on the measurement features a
//! self-paired ring leaves OFF (`docs/ring-features.md`). [`probe`] answers
//! "who owns this ring?" without changing anything.
//!
//! Neither function touches persistent storage or disconnects: the caller owns
//! the key (it must be saved BEFORE calling `pair`, so a crash mid-install never
//! loses the only copy of a key that is already live on the ring) and the
//! `Store` (reset the sync cursor when a key was installed).

use oura_protocol::auth::AuthResult;
use oura_protocol::device::{DeviceInfo, RingGeneration};
use oura_protocol::protocol::{capability, feature, feature_mode};

use crate::client::{auth_state_byte, OuraClient};
use crate::error::{Error, Result};
use crate::transport::Transport;

/// Which measurement features to turn on after the key is installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeaturePlan {
    /// Leave the ring as it is.
    None,
    /// DAYTIME_HR + SPO2 → AUTOMATIC: the stock consumer set. DAYTIME_HR produces
    /// the beat streams (0x60 night / 0x80 day) every HR-derived metric needs.
    Core,
    /// `Core` plus REAL_STEPS, EXERCISE_HR (best effort, server-flag gated on a
    /// stock ring) and RESTING_HR when it reads OFF.
    Full,
}

impl FeaturePlan {
    /// Parse a CLI/FFI spelling: `none` | `core` | `full`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" => Some(FeaturePlan::None),
            "core" => Some(FeaturePlan::Core),
            "full" => Some(FeaturePlan::Full),
            _ => None,
        }
    }
}

/// Progress stages reported while [`pair`] runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairStage {
    Identify,
    Probe,
    InstallKey,
    Verify,
    SyncTime,
    Battery,
    Features,
    Done,
}

impl PairStage {
    /// Short machine tag (used as the FFI progress `stage` string).
    pub fn tag(self) -> &'static str {
        match self {
            PairStage::Identify => "identify",
            PairStage::Probe => "probe",
            PairStage::InstallKey => "install_key",
            PairStage::Verify => "verify",
            PairStage::SyncTime => "time",
            PairStage::Battery => "battery",
            PairStage::Features => "features",
            PairStage::Done => "done",
        }
    }
}

/// Inputs to [`pair`].
#[derive(Clone, Copy, Debug)]
pub struct PairOptions {
    /// The 16-byte key to install (or re-install). Generate it with the platform
    /// CSPRNG and persist it before calling `pair`.
    pub key: [u8; 16],
    pub plan: FeaturePlan,
    /// Align the ring clock (both the `u64` and the app-style counter forms).
    pub sync_time: bool,
}

impl PairOptions {
    pub fn new(key: [u8; 16]) -> Self {
        Self {
            key,
            plan: FeaturePlan::Core,
            sync_time: true,
        }
    }
}

/// What happened to one feature in [`apply_feature_plan`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeatureResult {
    Set,
    AlreadySet,
    Rejected(String),
    Skipped(&'static str),
}

#[derive(Clone, Debug)]
pub struct FeatureOutcome {
    pub feature: u8,
    pub name: &'static str,
    pub mode: u8,
    pub result: FeatureResult,
}

/// Who holds this ring, as told by the auth verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ownership {
    /// No key installed: `SetAuthKey` will be accepted.
    FactoryReset,
    /// The candidate key authenticates: already paired with us.
    PairedWithThisKey,
    /// A different key is installed (the official app, or another host).
    OwnedElsewhere(AuthResult),
    /// The ring answered with a state byte we do not know, or did not answer the
    /// nonce request at all (`0xff`).
    Unknown(u8),
}

#[derive(Clone, Debug)]
pub struct ProbeReport {
    pub serial: String,
    pub hardware_id: Option<String>,
    pub generation: RingGeneration,
    pub firmware: Option<DeviceInfo>,
    pub ownership: Ownership,
}

#[derive(Clone, Debug)]
pub struct PairReport {
    pub serial: String,
    pub hardware_id: Option<String>,
    pub generation: RingGeneration,
    pub firmware: Option<DeviceInfo>,
    pub battery_pct: Option<u8>,
    /// Verdict after the install (always `Success` when `pair` returns `Ok`).
    pub auth: AuthResult,
    /// `true` when the ring was factory-reset and we installed `key`; `false`
    /// when the key was already live (a re-pair). Callers reset the sync cursor
    /// only in the first case.
    pub key_installed: bool,
    pub features: Vec<FeatureOutcome>,
}

/// Human name for a feature/capability id used by the plans.
pub fn feature_name(id: u8) -> &'static str {
    match id {
        feature::DAYTIME_HR => "daytime_hr",
        feature::EXERCISE_HR => "exercise_hr",
        feature::SPO2 => "spo2",
        feature::RESTING_HR => "resting_hr",
        capability::REAL_STEPS => "real_steps",
        capability::AMBIENT_LIGHT => "ambient",
        0x0d => "cva_ppg",
        _ => "feature",
    }
}

async fn identify<T: Transport>(
    client: &OuraClient<T>,
) -> Result<(String, Option<String>, RingGeneration, Option<DeviceInfo>)> {
    let serial = client.serial().await?;
    let hardware_id = client.hardware_id().await.ok();
    let generation = hardware_id
        .as_deref()
        .map(RingGeneration::from_hardware_id)
        .unwrap_or(RingGeneration::Unknown);
    let firmware = client.firmware().await.ok();
    Ok((serial, hardware_id, generation, firmware))
}

fn classify(state: std::result::Result<AuthResult, Error>) -> Result<Ownership> {
    match state {
        Ok(AuthResult::Success) => Ok(Ownership::PairedWithThisKey),
        Ok(AuthResult::InFactoryReset) => Ok(Ownership::FactoryReset),
        Ok(r @ AuthResult::AuthenticationError)
        | Ok(r @ AuthResult::NotOriginalOnboardedDevice) => Ok(Ownership::OwnedElsewhere(r)),
        Ok(AuthResult::Unknown(b)) => Ok(Ownership::Unknown(b)),
        // The ring did not answer the nonce request: some reset rings skip the
        // challenge entirely. Report it rather than fail the probe.
        Err(Error::NoAuthReply(_)) => Ok(Ownership::Unknown(0xff)),
        Err(e) => Err(e),
    }
}

/// Identify the ring and classify who owns it. `key` is the candidate key to
/// test (the stored one); with `None` an all-zero key is used, which only ever
/// answers `InFactoryReset` or a rejection.
pub async fn probe<T: Transport>(
    client: &OuraClient<T>,
    key: Option<&[u8; 16]>,
) -> Result<ProbeReport> {
    let (serial, hardware_id, generation, firmware) = identify(client).await?;
    let zero = [0u8; 16];
    let candidate = key.unwrap_or(&zero);
    let ownership = classify(client.auth_state(candidate).await)?;
    Ok(ProbeReport {
        serial,
        hardware_id,
        generation,
        firmware,
        ownership,
    })
}

/// Install `opts.key` on a factory-reset ring (or confirm it on a ring that
/// already holds it), verify, sync the clock, read the battery, and apply the
/// feature plan. See the module docs for what the caller still owns.
pub async fn pair<T: Transport>(
    client: &OuraClient<T>,
    opts: &PairOptions,
    mut on_stage: impl FnMut(PairStage),
) -> Result<PairReport> {
    on_stage(PairStage::Identify);
    let (serial, hardware_id, generation, firmware) = identify(client).await?;

    on_stage(PairStage::Probe);
    let install = match classify(client.auth_state(&opts.key).await)? {
        Ownership::FactoryReset => true,
        Ownership::PairedWithThisKey => false,
        Ownership::OwnedElsewhere(r) => {
            let reason = match r {
                AuthResult::AuthenticationError => "a different key is installed",
                AuthResult::NotOriginalOnboardedDevice => "bonded to another onboarding",
                _ => "not factory-reset",
            };
            return Err(Error::NotFactoryReset {
                state: auth_state_byte(r),
                reason,
            });
        }
        Ownership::Unknown(0xff) => {
            // No nonce answer: fall back to a blind install, which is what the
            // desktop client always did. The verify step below is the real check.
            tracing::warn!("ring did not answer the auth nonce; attempting SetAuthKey blindly");
            true
        }
        Ownership::Unknown(b) => {
            return Err(Error::NotFactoryReset {
                state: b,
                reason: "unexpected auth state",
            });
        }
    };

    let mut key_installed = false;
    if install {
        on_stage(PairStage::InstallKey);
        client.set_auth_key(&opts.key).await?;
        key_installed = true;
    }

    on_stage(PairStage::Verify);
    let auth = client.auth_state(&opts.key).await?;
    if !auth.is_success() {
        return Err(Error::Auth(format!(
            "key {} but the ring answered {auth:?} (state 0x{:02x}) on verification",
            if key_installed { "was installed" } else { "is already live" },
            auth_state_byte(auth)
        )));
    }

    if opts.sync_time {
        on_stage(PairStage::SyncTime);
        // Both forms: the second-precise u64 write and the app-parity counter
        // write. Best effort — a ring that ignores one still gets the other.
        if let Err(e) = client.sync_time().await {
            tracing::warn!("sync_time failed: {e}");
        }
        if let Err(e) = client.sync_time_app().await {
            tracing::warn!("sync_time_app failed: {e}");
        }
    }

    on_stage(PairStage::Battery);
    let battery_pct = client.battery().await.ok().map(|b| b.percent);

    on_stage(PairStage::Features);
    let features = apply_feature_plan(client, generation, opts.plan).await;

    on_stage(PairStage::Done);
    Ok(PairReport {
        serial,
        hardware_id,
        generation,
        firmware,
        battery_pct,
        auth,
        key_installed,
        features,
    })
}

async fn enable_automatic<T: Transport>(client: &OuraClient<T>, id: u8) -> FeatureOutcome {
    let already = matches!(
        client.feature_status(id).await,
        Ok(s) if s.mode == feature_mode::AUTOMATIC
    );
    let result = if already {
        FeatureResult::AlreadySet
    } else {
        match client.set_feature_mode(id, feature_mode::AUTOMATIC).await {
            Ok(()) => FeatureResult::Set,
            Err(e) => FeatureResult::Rejected(e.to_string()),
        }
    };
    FeatureOutcome {
        feature: id,
        name: feature_name(id),
        mode: feature_mode::AUTOMATIC,
        result,
    }
}

/// Turn on the features in `plan`, in the official app's enable order
/// (DAYTIME_HR → SPO2 → REAL_STEPS → EXERCISE_HR), never failing the pairing:
/// every outcome is reported, rejections included.
pub async fn apply_feature_plan<T: Transport>(
    client: &OuraClient<T>,
    generation: RingGeneration,
    plan: FeaturePlan,
) -> Vec<FeatureOutcome> {
    let ids: &[u8] = match plan {
        FeaturePlan::None => &[],
        FeaturePlan::Core => &[feature::DAYTIME_HR, feature::SPO2],
        FeaturePlan::Full => &[
            feature::DAYTIME_HR,
            feature::SPO2,
            capability::REAL_STEPS,
            feature::EXERCISE_HR,
        ],
    };
    if !generation.supports_feature_mode() {
        return ids
            .iter()
            .map(|&id| FeatureOutcome {
                feature: id,
                name: feature_name(id),
                mode: feature_mode::AUTOMATIC,
                result: FeatureResult::Skipped("ring generation <= 2 has no SetFeatureMode"),
            })
            .collect();
    }
    let mut out = Vec::with_capacity(ids.len() + 1);
    for &id in ids {
        out.push(enable_automatic(client, id).await);
    }
    if plan == FeaturePlan::Full {
        // The official app never toggles RESTING_HR (firmware-computed during
        // sleep); only touch it when it is explicitly OFF.
        let outcome = match client.feature_status(feature::RESTING_HR).await {
            Ok(s) if s.mode == feature_mode::OFF => enable_automatic(client, feature::RESTING_HR).await,
            Ok(_) => FeatureOutcome {
                feature: feature::RESTING_HR,
                name: feature_name(feature::RESTING_HR),
                mode: feature_mode::AUTOMATIC,
                result: FeatureResult::AlreadySet,
            },
            Err(e) => FeatureOutcome {
                feature: feature::RESTING_HR,
                name: feature_name(feature::RESTING_HR),
                mode: feature_mode::AUTOMATIC,
                result: FeatureResult::Rejected(e.to_string()),
            },
        };
        out.push(outcome);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::mock::MockTransport;
    use std::time::Duration;

    const KEY_HEX: &str = "4431967d8bacc2659743142b68391d9a";
    const NONCE_REQ: &str = "2f012b";
    const NONCE_RESP: &str = "2f102c0e2d6a0a08c99b4365f458e6e97382";
    const AUTH_REQ: &str = "2f112da38a8772d3acb6db5c2b516dd56987c8";

    fn key() -> [u8; 16] {
        hex::decode(KEY_HEX).unwrap().try_into().unwrap()
    }

    /// A mock that answers identity, nonce, battery and feature requests; the
    /// authenticate verdicts are supplied per test.
    fn ring(auth_verdicts: &[&[&str]]) -> MockTransport {
        let mock = MockTransport::new();
        mock.on("1803080010", &["191100585858585858585858585858"]);
        mock.on("1803180010", &["190700434f525f3035"]); // COR_05
        mock.on("0803000000", &["091202000003040301000105000cffeeddccbbaa"]);
        mock.on(NONCE_REQ, &[NONCE_RESP]);
        mock.on_sequence(AUTH_REQ, auth_verdicts);
        mock.on(&format!("2410{KEY_HEX}"), &["250100"]);
        mock.on_prefix("12", &[]);
        mock.on("0c00", &["0d0659000001f00f"]);
        // feature status: everything OFF; set → success
        for id in ["02", "04", "0b", "03", "08"] {
            mock.on(&format!("2f0220{id}"), &[&format!("2f0621{id}00000000")]);
            mock.on(&format!("2f0322{id}01"), &[&format!("2f0323{id}00")]);
        }
        mock
    }

    fn client(mock: MockTransport) -> OuraClient<MockTransport> {
        OuraClient::new(mock).with_quiet(Duration::from_millis(20))
    }

    fn writes_with_tag(client: &OuraClient<MockTransport>, tag: u8) -> Vec<Vec<u8>> {
        client
            .transport()
            .writes()
            .into_iter()
            .filter(|w| w.first() == Some(&tag))
            .collect()
    }

    #[tokio::test]
    async fn pairs_a_factory_reset_ring_and_enables_core_features() {
        let c = client(ring(&[&["2f022e02"], &["2f022e00"]]));
        let mut stages = Vec::new();
        let report = pair(&c, &PairOptions::new(key()), |s| stages.push(s))
            .await
            .unwrap();
        assert!(report.key_installed);
        assert_eq!(report.auth, AuthResult::Success);
        assert_eq!(report.serial, "XXXXXXXXXXXX");
        assert_eq!(report.generation, RingGeneration::Gen5);
        assert_eq!(report.battery_pct, Some(0x59));
        assert_eq!(report.features.len(), 2);
        assert!(report
            .features
            .iter()
            .all(|f| f.result == FeatureResult::Set));
        assert_eq!(stages.first(), Some(&PairStage::Identify));
        assert_eq!(stages.last(), Some(&PairStage::Done));
        assert!(stages.contains(&PairStage::InstallKey));

        // SetAuthKey sent exactly once, and before any SetFeatureMode.
        let writes = c.transport().writes();
        let key_writes: Vec<usize> = writes
            .iter()
            .enumerate()
            .filter(|(_, w)| w.first() == Some(&0x24))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(key_writes.len(), 1);
        let first_feature_write = writes
            .iter()
            .position(|w| w.starts_with(&[0x2f, 0x03, 0x22]))
            .unwrap();
        assert!(key_writes[0] < first_feature_write);
        // Both time-sync forms were sent.
        assert_eq!(writes_with_tag(&c, 0x12).len(), 2);
    }

    #[tokio::test]
    async fn repair_with_live_key_does_not_reinstall() {
        let c = client(ring(&[&["2f022e00"]]));
        let report = pair(&c, &PairOptions::new(key()), |_| {}).await.unwrap();
        assert!(!report.key_installed);
        assert!(writes_with_tag(&c, 0x24).is_empty());
    }

    #[tokio::test]
    async fn refuses_a_ring_owned_elsewhere() {
        let c = client(ring(&[&["2f022e01"]]));
        let err = pair(&c, &PairOptions::new(key()), |_| {}).await.unwrap_err();
        match err {
            Error::NotFactoryReset { state, .. } => assert_eq!(state, 0x01),
            other => panic!("unexpected error {other:?}"),
        }
        assert!(writes_with_tag(&c, 0x24).is_empty());
    }

    #[tokio::test]
    async fn surfaces_set_key_rejection() {
        let mock = ring(&[&["2f022e02"]]);
        mock.on(&format!("2410{KEY_HEX}"), &["250105"]);
        let c = client(mock);
        let err = pair(&c, &PairOptions::new(key()), |_| {}).await.unwrap_err();
        assert!(matches!(err, Error::SetKeyRejected(0x05)), "{err:?}");
    }

    #[tokio::test]
    async fn skips_features_on_old_generations() {
        let mock = ring(&[&["2f022e02"], &["2f022e00"]]);
        mock.on("1803180010", &["190700"]); // no hardware id → Unknown → attempt
        let c = client(mock);
        let report = pair(&c, &PairOptions::new(key()), |_| {}).await.unwrap();
        assert_eq!(report.generation, RingGeneration::Unknown);
        assert!(report.features.iter().all(|f| f.result == FeatureResult::Set));

        let out = apply_feature_plan(&c, RingGeneration::Gen3, FeaturePlan::Core).await;
        assert!(out.iter().all(|f| f.result == FeatureResult::Set));
        // A generation that cannot take SetFeatureMode is reported, not attempted.
        let mock2 = ring(&[&["2f022e00"]]);
        let c2 = client(mock2);
        let before = writes_with_tag(&c2, 0x2f).len();
        let out = apply_feature_plan(&c2, RingGeneration::Unknown, FeaturePlan::None).await;
        assert!(out.is_empty());
        assert_eq!(writes_with_tag(&c2, 0x2f).len(), before);
    }

    #[tokio::test]
    async fn full_plan_touches_resting_hr_only_when_off() {
        let mock = ring(&[&["2f022e00"]]);
        // RESTING_HR already AUTOMATIC
        mock.on("2f022008", &["2f06210801000000"]);
        let c = client(mock);
        let out = apply_feature_plan(&c, RingGeneration::Gen5, FeaturePlan::Full).await;
        let names: Vec<&str> = out.iter().map(|f| f.name).collect();
        assert_eq!(
            names,
            ["daytime_hr", "spo2", "real_steps", "exercise_hr", "resting_hr"]
        );
        assert_eq!(out[4].result, FeatureResult::AlreadySet);
        assert!(!c
            .transport()
            .writes()
            .iter()
            .any(|w| w.starts_with(&[0x2f, 0x03, 0x22, 0x08])));
    }

    #[tokio::test]
    async fn probe_classifies_every_verdict() {
        for (verdict, expected) in [
            ("2f022e00", Ownership::PairedWithThisKey),
            ("2f022e01", Ownership::OwnedElsewhere(AuthResult::AuthenticationError)),
            ("2f022e02", Ownership::FactoryReset),
            (
                "2f022e03",
                Ownership::OwnedElsewhere(AuthResult::NotOriginalOnboardedDevice),
            ),
            ("2f022e09", Ownership::Unknown(9)),
        ] {
            let c = client(ring(&[&[verdict]]));
            let report = probe(&c, Some(&key())).await.unwrap();
            assert_eq!(report.ownership, expected, "{verdict}");
            assert_eq!(report.hardware_id.as_deref(), Some("COR_05"));
        }
        // No nonce answer at all → Unknown(0xff), still a report.
        let mock = ring(&[&["2f022e00"]]);
        mock.on(NONCE_REQ, &[]);
        let c = client(mock);
        let report = probe(&c, None).await.unwrap();
        assert_eq!(report.ownership, Ownership::Unknown(0xff));
    }
}
