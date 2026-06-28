//! [`DevicePairing`]: the durable, device-scoped credential registry for the
//! local instance's control plane (phone-control handoff Part B deliverable 1).
//!
//! Part B turns the local instance into something a phone can drive. Before any
//! control traffic is trusted, a device must *pair*: it presents a one-time
//! pairing code (printed/returned at startup, NOT open) and receives a
//! per-device bearer token. Every later request presents that token; the
//! registry verifies it, and the instance owner can REVOKE a device so its token
//! stops verifying. This is the trust boundary, so the rules are strict:
//!
//! * **Hashed at rest.** The raw token is returned to the caller exactly once (at
//!   pairing) and is NEVER stored. The registry persists only a SHA-256 hash, so
//!   a leaked registry file cannot be replayed as a credential.
//! * **Constant-time verification.** A presented token is hashed and compared to
//!   the stored hash with [`subtle::ConstantTimeEq`], so verification time does
//!   not leak how many leading bytes matched.
//! * **Device-scoped + revocable.** Each device has its own id, label, secret
//!   hash, paired-at timestamp, and a `revoked` flag. Revoking flips the flag;
//!   the device record stays for audit but its token no longer verifies.
//! * **Durable.** The registry is a small serde-json file under the sidecar dir
//!   (`<sidecar>/control/devices.json`). A simple file is the simpler durable
//!   option here than a second `RedCoreGraphStore`: the ingest sink already holds
//!   a process-level lock on the sidecar `graph` dir (a second graph handle would
//!   contend for it), and the registry is a tiny append-mostly list. The file is
//!   written atomically (temp file + rename) so a crash mid-write cannot corrupt
//!   it.
//!
//! Nothing here logs a raw secret or a pairing code.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rustyred_thg_core::now_ms;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::Result;

/// Sidecar-relative directory for control-plane state.
const CONTROL_DIR: &str = "control";
/// File name (under `<sidecar>/control/`) holding the device registry.
const DEVICES_FILE: &str = "devices.json";
/// File name (under `<sidecar>/control/`) holding the local-access device's token
/// so the desktop (Tauri) shell can read it and hand it to the same-machine
/// webapp as its `Authorization: Bearer` credential.
const LOCAL_ACCESS_FILE: &str = "local_access.json";
/// Device label stamped on the auto-provisioned local-access device, so it is
/// recognizable in `list_devices` (and revocable) like any other device.
pub const LOCAL_ACCESS_LABEL: &str = "local_access";

/// Bytes of entropy in a freshly issued device token (and the one-time pairing
/// code). 32 bytes = 256 bits, rendered as 64 lowercase hex chars.
const TOKEN_ENTROPY_BYTES: usize = 32;

/// A single paired device's persisted record. The raw token is never stored;
/// `secret_hash` is the hex SHA-256 of the issued token.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PairedDevice {
    /// Stable per-device id (also the bearer-token's public prefix, see
    /// [`DeviceToken`]). Opaque, server-issued.
    pub device_id: String,
    /// Human-facing label supplied at pairing time (e.g. "Travis iPhone").
    pub label: String,
    /// Hex SHA-256 of the issued token. The raw token is returned once and never
    /// persisted; this is what a presented token is hashed-and-compared against.
    pub secret_hash: String,
    /// Wall-clock pairing time (epoch ms).
    pub paired_at_ms: i64,
    /// Whether this device has been revoked. A revoked device's token no longer
    /// verifies; the record is kept for audit.
    #[serde(default)]
    pub revoked: bool,
    /// Wall-clock revocation time (epoch ms), set when `revoked` flips true.
    #[serde(default)]
    pub revoked_at_ms: Option<i64>,
}

impl PairedDevice {
    /// A public, secret-free view for listing (the `secret_hash` is intentionally
    /// omitted so list responses cannot leak even the hash).
    pub fn summary(&self) -> DeviceSummary {
        DeviceSummary {
            device_id: self.device_id.clone(),
            label: self.label.clone(),
            paired_at_ms: self.paired_at_ms,
            revoked: self.revoked,
            revoked_at_ms: self.revoked_at_ms,
        }
    }
}

/// A secret-free device record for listing over the API.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeviceSummary {
    pub device_id: String,
    pub label: String,
    pub paired_at_ms: i64,
    pub revoked: bool,
    pub revoked_at_ms: Option<i64>,
}

/// The result of pairing a new device: the durable summary plus the raw bearer
/// token, returned to the caller EXACTLY ONCE. The token is not stored anywhere;
/// if the caller loses it, the device must be revoked and re-paired.
#[derive(Clone, Debug)]
pub struct PairingResult {
    /// The newly paired device (secret-free).
    pub device: DeviceSummary,
    /// The raw bearer token the device must present on later requests. Surfaced
    /// once; never persisted. Format: `<device_id>.<hex-secret>` (see
    /// [`DeviceToken`]).
    pub token: String,
}

/// On-disk shape of the registry file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct DeviceRegistryFile {
    /// device_id -> record.
    devices: BTreeMap<String, PairedDevice>,
}

/// The persisted local-access record: the auto-provisioned local-access device's
/// id and its raw bearer token.
///
/// Unlike the device registry (which stores only token HASHES), this file holds
/// the RAW token on purpose: it is the local-first credential the desktop (Tauri)
/// shell reads and hands to the same-machine webapp as `Authorization: Bearer`.
/// It is no more sensitive than the registry itself -- both live inside the
/// sidecar control dir (always gitignored, same trust domain), and the token only
/// authorizes the LOOPBACK control endpoint. Revoke it (by `device_id`) to cut
/// the webapp off, exactly like any other device.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalAccess {
    /// The local-access device's id (revoke it via
    /// [`DevicePairing::revoke_device`]).
    pub device_id: String,
    /// The raw bearer token for the same-machine webapp.
    pub token: String,
}

/// A presented bearer token, split into its public device-id prefix and the raw
/// secret. The token wire format is `<device_id>.<hex-secret>`; carrying the
/// device id in the token lets verification look up exactly one candidate record
/// (and still compare the secret in constant time) instead of scanning every
/// device.
struct DeviceToken<'a> {
    device_id: &'a str,
    secret: &'a str,
}

impl<'a> DeviceToken<'a> {
    /// Parse a presented token. Returns `None` for any malformed token (no
    /// separator, empty half), so a garbage token is a clean auth failure.
    fn parse(raw: &'a str) -> Option<Self> {
        let (device_id, secret) = raw.split_once('.')?;
        if device_id.is_empty() || secret.is_empty() {
            return None;
        }
        Some(Self { device_id, secret })
    }
}

/// Hex SHA-256 of the given bytes. Used to hash tokens and pairing codes at rest.
fn sha256_hex(input: &[u8]) -> String {
    let digest = Sha256::digest(input);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        // Infallible into a String.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Generate `n_bytes` of CSPRNG randomness as a lowercase hex string (length
/// `2 * n_bytes`). Uses `getrandom` (the OS CSPRNG) so the output is
/// unpredictable; returns an error rather than a weak value if the OS source is
/// unavailable. Reused for device secrets, device ids, and pairing codes.
pub(crate) fn random_token_hex(n_bytes: usize) -> Result<String> {
    let mut bytes = vec![0u8; n_bytes];
    getrandom::fill(&mut bytes).map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
        format!("control-plane CSPRNG unavailable: {error}").into()
    })?;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    Ok(out)
}

/// A device-secret-sized hex token (`TOKEN_ENTROPY_BYTES` of entropy).
fn random_hex() -> Result<String> {
    random_token_hex(TOKEN_ENTROPY_BYTES)
}

/// Outcome of a token verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyOutcome {
    /// The token matched an active (non-revoked) device.
    Authorized,
    /// No active device matched (unknown device, wrong secret, or revoked).
    Rejected,
}

impl VerifyOutcome {
    pub fn is_authorized(self) -> bool {
        matches!(self, VerifyOutcome::Authorized)
    }
}

/// The durable device-pairing registry. Pair a device to issue a token, verify a
/// presented token in constant time, list devices, and revoke a device. All
/// mutations persist to the sidecar JSON file before returning.
///
/// Cloneable handle: the in-memory state is behind an `Arc<Mutex<..>>`, so the
/// HTTP layer can share one registry across handlers cheaply.
#[derive(Clone)]
pub struct DevicePairing {
    inner: std::sync::Arc<DevicePairingInner>,
}

struct DevicePairingInner {
    /// Where the registry persists.
    path: PathBuf,
    /// In-memory mirror of the file; the file is the source of truth on open and
    /// is rewritten on every mutation.
    state: Mutex<DeviceRegistryFile>,
}

impl DevicePairing {
    /// Open (or create) the registry under the given sidecar directory. The file
    /// lives at `<sidecar_dir>/control/devices.json`; the directory is created if
    /// absent and an existing file is loaded.
    pub fn open(sidecar_dir: &Path) -> Result<Self> {
        let dir = sidecar_dir.join(CONTROL_DIR);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(DEVICES_FILE);
        let state = if path.is_file() {
            let bytes = std::fs::read(&path)?;
            // A corrupt/empty file is a hard error rather than a silent reset:
            // silently dropping paired devices would be a security regression
            // (every device would have to re-pair, and a tampered file would be
            // masked). The operator can inspect/restore it.
            serde_json::from_slice::<DeviceRegistryFile>(&bytes)
                .map_err(|error| format!("parse device registry {}: {error}", path.display()))?
        } else {
            DeviceRegistryFile::default()
        };
        Ok(Self {
            inner: std::sync::Arc::new(DevicePairingInner {
                path,
                state: Mutex::new(state),
            }),
        })
    }

    /// Pair a new device under `label`, issuing a fresh bearer token. The raw
    /// token is returned ONCE (in [`PairingResult::token`]); only its hash is
    /// persisted. Each call creates a distinct device with its own id + secret.
    pub fn pair_device(&self, label: impl Into<String>) -> Result<PairingResult> {
        let label = label.into();
        let device_id = format!("dev_{}", &random_hex()?[..16]);
        let secret = random_hex()?;
        let token = format!("{device_id}.{secret}");
        let secret_hash = sha256_hex(secret.as_bytes());

        let device = PairedDevice {
            device_id: device_id.clone(),
            label,
            secret_hash,
            paired_at_ms: now_ms(),
            revoked: false,
            revoked_at_ms: None,
        };
        let summary = device.summary();

        {
            let mut state = self.lock_state();
            state.devices.insert(device_id, device);
            self.persist(&state)?;
        }

        Ok(PairingResult {
            device: summary,
            token,
        })
    }

    /// Verify a presented bearer token in constant time. Returns
    /// [`VerifyOutcome::Authorized`] only when the token parses, names a known
    /// device, the device is not revoked, and the secret hash matches. Any other
    /// case (malformed, unknown device, revoked, wrong secret) is `Rejected`.
    ///
    /// The secret comparison is constant-time over the hex hash bytes; a revoked
    /// or unknown device is still hashed-and-compared against a placeholder so the
    /// "device exists" branch does not become a timing oracle for which device
    /// ids are registered.
    pub fn verify(&self, presented: &str) -> VerifyOutcome {
        let token = match DeviceToken::parse(presented) {
            Some(token) => token,
            None => return VerifyOutcome::Rejected,
        };
        let presented_hash = sha256_hex(token.secret.as_bytes());

        let state = self.lock_state();
        let device = state.devices.get(token.device_id);
        // Choose the hash to compare against: the device's stored hash if it
        // exists and is active, else a fixed placeholder of the same length. We
        // always run the constant-time compare so the unknown/revoked path costs
        // the same as the wrong-secret path.
        let (expected_hash, active) = match device {
            Some(record) if !record.revoked => (record.secret_hash.as_str(), true),
            _ => (PLACEHOLDER_HASH, false),
        };
        let matches = constant_time_str_eq(&presented_hash, expected_hash);
        if active && matches {
            VerifyOutcome::Authorized
        } else {
            VerifyOutcome::Rejected
        }
    }

    /// List every device (including revoked ones, for audit), secret-free.
    /// Ordered by device id for a stable response.
    pub fn list_devices(&self) -> Vec<DeviceSummary> {
        let state = self.lock_state();
        state.devices.values().map(PairedDevice::summary).collect()
    }

    /// Revoke a device by id: its token stops verifying immediately and the
    /// change is persisted. Returns `Ok(true)` if a device was revoked, `Ok(false)`
    /// if no such device id exists (idempotent: revoking an already-revoked device
    /// returns `true` and is a no-op write-through). Errors only on a persist
    /// failure.
    pub fn revoke_device(&self, device_id: &str) -> Result<bool> {
        let mut state = self.lock_state();
        let Some(device) = state.devices.get_mut(device_id) else {
            return Ok(false);
        };
        if !device.revoked {
            device.revoked = true;
            device.revoked_at_ms = Some(now_ms());
        }
        self.persist(&state)?;
        Ok(true)
    }

    /// Number of devices on record (active + revoked).
    pub fn device_count(&self) -> usize {
        self.lock_state().devices.len()
    }

    /// Ensure a `local_access` device exists and return its bearer token.
    ///
    /// This is the local-first connection path: a same-machine desktop webapp on
    /// loopback should NOT have to do the QR/pairing-code dance. The instance
    /// auto-provisions one device labelled [`LOCAL_ACCESS_LABEL`], persists its
    /// token at `<sidecar>/control/local_access.json` (so the Tauri shell can read
    /// it via [`local_access_token`](Self::local_access_token) and hand it to the
    /// webapp), and verifies it through the SAME [`DevicePairing`] path as any
    /// paired device -- so it is one auth model and is revocable like any device
    /// (revoke it by id via [`revoke_device`](Self::revoke_device)).
    ///
    /// Idempotent and self-healing: if the persisted token is present AND still
    /// verifies, it is reused; if the file is missing, or the device was revoked /
    /// removed (so the token no longer verifies), a fresh local-access device is
    /// minted and persisted. Call this on startup.
    pub fn ensure_local_access(&self) -> Result<LocalAccess> {
        // Reuse the persisted token if it still authorizes through the normal path.
        if let Some(existing) = self.read_local_access()? {
            if self.verify(&existing.token).is_authorized() {
                return Ok(existing);
            }
        }
        // Mint a fresh local-access device (goes through the same pairing path) and
        // persist its token for the Tauri shell to read.
        let issued = self.pair_device(LOCAL_ACCESS_LABEL)?;
        let access = LocalAccess {
            device_id: issued.device.device_id,
            token: issued.token,
        };
        self.write_local_access(&access)?;
        Ok(access)
    }

    /// Read the persisted local-access token, if one has been provisioned. This is
    /// what the desktop (Tauri) backend reads to hand the same-machine webapp its
    /// bearer token. Returns `Ok(None)` if [`ensure_local_access`](Self::ensure_local_access)
    /// has never run (no file yet). Does NOT verify the token still authorizes --
    /// the caller can do that or just call `ensure_local_access` (which self-heals).
    pub fn local_access_token(&self) -> Result<Option<String>> {
        Ok(self.read_local_access()?.map(|access| access.token))
    }

    /// Path to the local-access token file.
    fn local_access_path(&self) -> Result<PathBuf> {
        let dir = self
            .inner
            .path
            .parent()
            .ok_or("device registry path has no parent directory")?;
        Ok(dir.join(LOCAL_ACCESS_FILE))
    }

    /// Read the persisted local-access record, or `None` if the file is absent. A
    /// corrupt file is a hard error (not a silent reset), like the device registry.
    fn read_local_access(&self) -> Result<Option<LocalAccess>> {
        let path = self.local_access_path()?;
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        let access = serde_json::from_slice::<LocalAccess>(&bytes)
            .map_err(|error| format!("parse local access {}: {error}", path.display()))?;
        Ok(Some(access))
    }

    /// Write the local-access record atomically (temp file + rename), mirroring the
    /// device-registry persist so a crash mid-write keeps the prior good file.
    fn write_local_access(&self, access: &LocalAccess) -> Result<()> {
        let path = self.local_access_path()?;
        let dir = path
            .parent()
            .ok_or("local access path has no parent directory")?;
        let json = serde_json::to_vec_pretty(access)?;
        let tmp = dir.join(format!(".{LOCAL_ACCESS_FILE}.tmp"));
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, DeviceRegistryFile> {
        // A poisoned lock means a prior holder panicked mid-mutation; recover the
        // guard rather than cascading the panic. The on-disk file is still the
        // source of truth and is rewritten atomically on the next mutation.
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Write the registry to disk atomically (temp file in the same dir + rename),
    /// so a crash mid-write leaves the previous good file intact.
    fn persist(&self, state: &DeviceRegistryFile) -> Result<()> {
        let json = serde_json::to_vec_pretty(state)?;
        let dir = self
            .inner
            .path
            .parent()
            .ok_or("device registry path has no parent directory")?;
        let tmp = dir.join(format!(".{DEVICES_FILE}.tmp"));
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, &self.inner.path)?;
        Ok(())
    }
}

/// A fixed-length placeholder hash (hex SHA-256 of the empty input) compared
/// against when no active device matches, so verification of an unknown/revoked
/// device runs the same constant-time compare as a wrong secret and does not leak
/// device existence through timing. (This specific value is not a secret.)
const PLACEHOLDER_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Constant-time string equality. Used for both the token-hash comparison and the
/// one-time pairing-code comparison so neither leaks how many leading bytes
/// matched through timing. Differing lengths return false (the length of a secret
/// is not itself the secret); equal-length inputs are compared byte-for-byte in
/// constant time via [`subtle::ConstantTimeEq`].
pub(crate) fn constant_time_str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_registry() -> (tempfile::TempDir, DevicePairing) {
        let dir = tempfile::tempdir().unwrap();
        let registry = DevicePairing::open(dir.path()).unwrap();
        (dir, registry)
    }

    #[test]
    fn pairing_issues_a_usable_token() {
        let (_dir, registry) = temp_registry();
        let result = registry.pair_device("Test Phone").unwrap();
        assert!(
            registry.verify(&result.token).is_authorized(),
            "the issued token must authorize"
        );
        assert_eq!(result.device.label, "Test Phone");
        assert!(!result.device.revoked);
    }

    #[test]
    fn raw_secret_is_never_persisted() {
        let (dir, registry) = temp_registry();
        let result = registry.pair_device("Phone").unwrap();
        // The secret half of the token must not appear anywhere in the file.
        let secret = result.token.split_once('.').unwrap().1;
        let file = dir.path().join(CONTROL_DIR).join(DEVICES_FILE);
        let contents = std::fs::read_to_string(&file).unwrap();
        assert!(
            !contents.contains(secret),
            "raw secret must not be stored on disk"
        );
        // The hash of the secret IS stored.
        assert!(contents.contains(&sha256_hex(secret.as_bytes())));
    }

    #[test]
    fn garbage_and_missing_tokens_are_rejected() {
        let (_dir, registry) = temp_registry();
        registry.pair_device("Phone").unwrap();
        assert!(!registry.verify("").is_authorized());
        assert!(!registry.verify("not-a-token").is_authorized());
        assert!(!registry.verify("dev_unknown.deadbeef").is_authorized());
        // Right device id, wrong secret.
        let id = registry.list_devices()[0].device_id.clone();
        assert!(!registry.verify(&format!("{id}.wrongsecret")).is_authorized());
    }

    #[test]
    fn revoked_device_no_longer_verifies() {
        let (_dir, registry) = temp_registry();
        let result = registry.pair_device("Phone").unwrap();
        assert!(registry.verify(&result.token).is_authorized());

        assert!(registry.revoke_device(&result.device.device_id).unwrap());
        assert!(
            !registry.verify(&result.token).is_authorized(),
            "a revoked device's token must stop verifying"
        );
        // Revoking an unknown id is a clean false.
        assert!(!registry.revoke_device("dev_nope").unwrap());
        // The revoked device is still listed (for audit), flagged revoked.
        let listed = registry.list_devices();
        let device = listed
            .iter()
            .find(|d| d.device_id == result.device.device_id)
            .unwrap();
        assert!(device.revoked);
        assert!(device.revoked_at_ms.is_some());
    }

    #[test]
    fn registry_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let token = {
            let registry = DevicePairing::open(dir.path()).unwrap();
            let kept = registry.pair_device("Kept").unwrap();
            let revoked = registry.pair_device("Revoked").unwrap();
            registry.revoke_device(&revoked.device.device_id).unwrap();
            kept.token
        };
        // Reopen from the same sidecar dir: the active token still verifies and
        // the revoked device is still revoked.
        let reopened = DevicePairing::open(dir.path()).unwrap();
        assert_eq!(reopened.device_count(), 2);
        assert!(
            reopened.verify(&token).is_authorized(),
            "an active device must survive a registry reopen"
        );
        let revoked_count = reopened
            .list_devices()
            .iter()
            .filter(|d| d.revoked)
            .count();
        assert_eq!(revoked_count, 1, "revocation must survive a reopen");
    }

    #[test]
    fn each_pairing_is_device_scoped_and_distinct() {
        let (_dir, registry) = temp_registry();
        let a = registry.pair_device("A").unwrap();
        let b = registry.pair_device("B").unwrap();
        assert_ne!(a.device.device_id, b.device.device_id);
        assert_ne!(a.token, b.token);
        // Revoking A leaves B working.
        registry.revoke_device(&a.device.device_id).unwrap();
        assert!(!registry.verify(&a.token).is_authorized());
        assert!(registry.verify(&b.token).is_authorized());
    }

    #[test]
    fn local_access_provisions_persists_and_authorizes() {
        let (dir, registry) = temp_registry();
        // First call mints + persists a local-access device.
        let access = registry.ensure_local_access().unwrap();
        assert!(
            registry.verify(&access.token).is_authorized(),
            "the local-access token authorizes through the normal pairing path"
        );
        // The local-access device is listed (so it is auditable/revocable) with the
        // recognizable label.
        let listed = registry.list_devices();
        let device = listed
            .iter()
            .find(|d| d.device_id == access.device_id)
            .expect("local-access device is in the registry");
        assert_eq!(device.label, LOCAL_ACCESS_LABEL);

        // The token file the Tauri shell reads exists and carries the SAME token.
        let token_for_tauri = registry
            .local_access_token()
            .unwrap()
            .expect("a local-access token has been provisioned");
        assert_eq!(token_for_tauri, access.token);
        let file = dir.path().join(CONTROL_DIR).join(LOCAL_ACCESS_FILE);
        assert!(file.is_file(), "the local-access token file is persisted");
    }

    #[test]
    fn local_access_is_idempotent_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let token = {
            let registry = DevicePairing::open(dir.path()).unwrap();
            let first = registry.ensure_local_access().unwrap();
            // A second call reuses the SAME token (does not mint a new device).
            let second = registry.ensure_local_access().unwrap();
            assert_eq!(first.token, second.token, "ensure_local_access is idempotent");
            assert_eq!(registry.device_count(), 1, "no duplicate local-access device");
            first.token
        };
        // Reopen: the persisted token still verifies and is still readable.
        let reopened = DevicePairing::open(dir.path()).unwrap();
        assert!(reopened.verify(&token).is_authorized());
        assert_eq!(reopened.local_access_token().unwrap(), Some(token));
    }

    #[test]
    fn local_access_is_revocable_and_self_heals() {
        let (_dir, registry) = temp_registry();
        let access = registry.ensure_local_access().unwrap();
        // Revoke it like any device: the token stops verifying.
        assert!(registry.revoke_device(&access.device_id).unwrap());
        assert!(
            !registry.verify(&access.token).is_authorized(),
            "a revoked local-access token stops verifying (revocable like any device)"
        );
        // ensure_local_access self-heals: the revoked token no longer verifies, so a
        // fresh local-access device is minted with a new, working token.
        let healed = registry.ensure_local_access().unwrap();
        assert_ne!(healed.token, access.token, "a new local-access token is issued");
        assert!(registry.verify(&healed.token).is_authorized());
        assert_ne!(healed.device_id, access.device_id);
    }
}
