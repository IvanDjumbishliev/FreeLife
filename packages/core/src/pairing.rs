//! PIN-confirmed pairing.
//!
//! Pairing runs over a plain TCP stream, before any trust exists. Both
//! devices show a 6-digit code derived from an X25519 shared secret; the user
//! checks the codes match. On success each side stores the other's identity
//! key in the [`TrustStore`], and later sessions authenticate with it over
//! TLS 1.3.
//!
//! ```text
//! Initiator                                     Responder
//!   Request  { version, device_I, commit } ──▶
//!                                         ◀──  Response { version, device_R, eph_R, nonce_R }
//!   Reveal   { eph_I, nonce_I }            ──▶  (checks commit == H(eph_I, nonce_I))
//!        both derive code = HKDF(X25519(eph_I, eph_R), transcript)
//!        both users confirm the codes match
//!   Confirm  { sig_I(transcript) }         ──▶
//!                                         ◀──  Confirm  { sig_R(transcript) }
//! ```
//!
//! Why the commitment: with a plain exchange, a man in the middle who sees
//! one side's key could try ~10⁶ keys of its own until both sides' codes
//! collide. Committing to `eph_I` before seeing `eph_R` takes that choice
//! away, so an attacker gets one guess with a 1-in-10⁶ chance.
//!
//! Why the signatures: they prove each side holds the private key behind the
//! `public_key` in its `DeviceInfo`, so nobody can pair while claiming
//! another device's identity.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use ring::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, X25519};
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::identity::verify_signature;
use crate::protocol::{encode, read_frame, write_frame};
use crate::{DeviceInfo, Error, Identity, PairingError, Result, TrustStore, PROTOCOL_VERSION};

const COMMIT_CONTEXT: &[u8] = b"freelife-pairing-commit-v1";
const TRANSCRIPT_CONTEXT: &[u8] = b"freelife-pairing-transcript-v1";
const CODE_CONTEXT: &[u8] = b"freelife-pairing-code-v1";
const CONFIRM_CONTEXT: &[u8] = b"freelife-pairing-confirm-v1";

type Key32 = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingRequest {
    pub version: u16,
    pub device: DeviceInfo,
    /// `SHA-256(COMMIT_CONTEXT ‖ ephemeral_key ‖ nonce)` of the initiator's later [`PairingMessage::Reveal`].
    pub commitment: Key32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingResponse {
    pub version: u16,
    pub device: DeviceInfo,
    pub ephemeral_key: Key32,
    pub nonce: Key32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairingMessage {
    Request(PairingRequest),
    Response(PairingResponse),
    Reveal { ephemeral_key: Key32, nonce: Key32 },
    Confirm { signature: Vec<u8> },
    Reject,
}

/// The 6-digit code both users compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PairingCode(u32);

impl PairingCode {
    pub fn value(self) -> u32 {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn for_test(value: u32) -> Self {
        Self(value)
    }
}

impl fmt::Display for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:06}", self.0)
    }
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Asks the user whether the code shown on this device matches the other one.
///
/// Implemented for any `Fn(DeviceInfo, PairingCode) -> impl Future<Output = bool>`.
pub trait PinConfirmer: Send + Sync {
    fn confirm(&self, peer: DeviceInfo, code: PairingCode) -> BoxFuture<'_, bool>;
}

impl<F, Fut> PinConfirmer for F
where
    F: Fn(DeviceInfo, PairingCode) -> Fut + Send + Sync,
    Fut: Future<Output = bool> + Send + 'static,
{
    fn confirm(&self, peer: DeviceInfo, code: PairingCode) -> BoxFuture<'_, bool> {
        Box::pin(self(peer, code))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingOutcome {
    /// The newly trusted device.
    pub peer: DeviceInfo,
    pub code: PairingCode,
}

#[derive(Clone, Copy)]
enum Role {
    Initiator,
    Responder,
}

impl Role {
    fn tag(self) -> u8 {
        match self {
            Role::Initiator => 1,
            Role::Responder => 2,
        }
    }
}

/// Pair with the device on the other end of `stream`, which must be running
/// [`pair_as_responder`].
pub async fn pair_as_initiator<S>(
    stream: &mut S,
    identity: &Identity,
    local: &DeviceInfo,
    trust: &TrustStore,
    confirmer: &dyn PinConfirmer,
) -> Result<PairingOutcome>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    check_local(identity, local)?;
    let (eph_secret, eph_public) = ephemeral_keypair()?;
    let nonce = random_32()?;

    let request = PairingRequest {
        version: PROTOCOL_VERSION,
        device: local.clone(),
        commitment: commitment(&eph_public, &nonce),
    };
    write_frame(stream, &PairingMessage::Request(request.clone())).await?;

    let response = match read_frame(stream).await? {
        PairingMessage::Response(r) => r,
        PairingMessage::Reject => return Err(PairingError::RejectedByPeer.into()),
        _ => return Err(PairingError::UnexpectedMessage.into()),
    };
    check_peer(response.version, &response.device, local)?;

    write_frame(
        stream,
        &PairingMessage::Reveal {
            ephemeral_key: eph_public,
            nonce,
        },
    )
    .await?;

    let transcript = transcript(&request, &response, &eph_public, &nonce)?;
    let shared = agree(eph_secret, &response.ephemeral_key)?;
    let code = derive_code(&shared, &transcript)?;

    if !confirmer.confirm(response.device.clone(), code).await {
        write_frame(stream, &PairingMessage::Reject).await?;
        return Err(PairingError::RejectedLocally.into());
    }
    let signature = identity.sign(&confirm_payload(Role::Initiator, &transcript));
    write_frame(stream, &PairingMessage::Confirm { signature }).await?;

    expect_confirm(stream, &response.device, Role::Responder, &transcript).await?;

    trust.trust(&response.device).await?;
    Ok(PairingOutcome {
        peer: response.device,
        code,
    })
}

/// Answer a pairing request from the device on the other end of `stream`.
pub async fn pair_as_responder<S>(
    stream: &mut S,
    identity: &Identity,
    local: &DeviceInfo,
    trust: &TrustStore,
    confirmer: &dyn PinConfirmer,
) -> Result<PairingOutcome>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    check_local(identity, local)?;
    let request = match read_frame(stream).await? {
        PairingMessage::Request(r) => r,
        _ => return Err(PairingError::UnexpectedMessage.into()),
    };
    if let Err(e) = check_peer(request.version, &request.device, local) {
        let _ = write_frame(stream, &PairingMessage::Reject).await;
        return Err(e);
    }

    let (eph_secret, eph_public) = ephemeral_keypair()?;
    let response = PairingResponse {
        version: PROTOCOL_VERSION,
        device: local.clone(),
        ephemeral_key: eph_public,
        nonce: random_32()?,
    };
    write_frame(stream, &PairingMessage::Response(response.clone())).await?;

    let (peer_eph, peer_nonce) = match read_frame(stream).await? {
        PairingMessage::Reveal {
            ephemeral_key,
            nonce,
        } => (ephemeral_key, nonce),
        PairingMessage::Reject => return Err(PairingError::RejectedByPeer.into()),
        _ => return Err(PairingError::UnexpectedMessage.into()),
    };
    if commitment(&peer_eph, &peer_nonce) != request.commitment {
        let _ = write_frame(stream, &PairingMessage::Reject).await;
        return Err(PairingError::CommitmentMismatch.into());
    }

    let transcript = transcript(&request, &response, &peer_eph, &peer_nonce)?;
    let shared = agree(eph_secret, &peer_eph)?;
    let code = derive_code(&shared, &transcript)?;

    if !confirmer.confirm(request.device.clone(), code).await {
        write_frame(stream, &PairingMessage::Reject).await?;
        return Err(PairingError::RejectedLocally.into());
    }

    expect_confirm(stream, &request.device, Role::Initiator, &transcript).await?;
    let signature = identity.sign(&confirm_payload(Role::Responder, &transcript));
    write_frame(stream, &PairingMessage::Confirm { signature }).await?;

    trust.trust(&request.device).await?;
    Ok(PairingOutcome {
        peer: request.device,
        code,
    })
}

async fn expect_confirm<S>(
    stream: &mut S,
    peer: &DeviceInfo,
    peer_role: Role,
    transcript: &Key32,
) -> Result<()>
where
    S: AsyncRead + Unpin,
{
    match read_frame(stream).await? {
        PairingMessage::Confirm { signature } => {
            let payload = confirm_payload(peer_role, transcript);
            if verify_signature(&peer.public_key, &payload, &signature) {
                Ok(())
            } else {
                Err(PairingError::BadSignature.into())
            }
        }
        PairingMessage::Reject => Err(PairingError::RejectedByPeer.into()),
        _ => Err(PairingError::UnexpectedMessage.into()),
    }
}

fn check_local(identity: &Identity, local: &DeviceInfo) -> Result<()> {
    if local.public_key != identity.public_key() {
        return Err(Error::InvalidDeviceInfo(
            "local device info does not match identity",
        ));
    }
    local.validate()
}

fn check_peer(version: u16, peer: &DeviceInfo, local: &DeviceInfo) -> Result<()> {
    if version != PROTOCOL_VERSION {
        return Err(PairingError::UnsupportedVersion(version).into());
    }
    if peer.validate().is_err() {
        return Err(PairingError::InvalidPeer("device info failed validation").into());
    }
    if peer.id == local.id {
        return Err(PairingError::InvalidPeer("cannot pair with self").into());
    }
    Ok(())
}

fn random_32() -> Result<Key32> {
    let mut out = [0u8; 32];
    SystemRandom::new()
        .fill(&mut out)
        .map_err(|_| Error::Crypto("system RNG failed"))?;
    Ok(out)
}

fn ephemeral_keypair() -> Result<(EphemeralPrivateKey, Key32)> {
    let secret = EphemeralPrivateKey::generate(&X25519, &SystemRandom::new())
        .map_err(|_| Error::Crypto("X25519 key generation failed"))?;
    let public = secret
        .compute_public_key()
        .map_err(|_| Error::Crypto("X25519 public key failed"))?;
    let public: Key32 = public
        .as_ref()
        .try_into()
        .map_err(|_| Error::Crypto("unexpected X25519 key length"))?;
    Ok((secret, public))
}

fn agree(secret: EphemeralPrivateKey, peer_public: &Key32) -> Result<Key32> {
    agreement::agree_ephemeral(secret, &UnparsedPublicKey::new(&X25519, peer_public), |s| {
        let mut out = [0u8; 32];
        out.copy_from_slice(s);
        out
    })
    // ring rejects low-order peer keys that would yield an all-zero secret.
    .map_err(|_| PairingError::InvalidPeer("invalid ephemeral key").into())
}

fn commitment(ephemeral_key: &Key32, nonce: &Key32) -> Key32 {
    let mut h = Sha256::new();
    h.update(COMMIT_CONTEXT);
    h.update(ephemeral_key);
    h.update(nonce);
    h.finalize().into()
}

/// Hash of everything both sides saw, binding the code and signatures to
/// this exact exchange (both identities, both ephemeral keys, both nonces).
fn transcript(
    request: &PairingRequest,
    response: &PairingResponse,
    initiator_eph: &Key32,
    initiator_nonce: &Key32,
) -> Result<Key32> {
    let mut h = Sha256::new();
    h.update(TRANSCRIPT_CONTEXT);
    for part in [encode(request)?, encode(response)?] {
        h.update((part.len() as u32).to_be_bytes());
        h.update(&part);
    }
    h.update(initiator_eph);
    h.update(initiator_nonce);
    Ok(h.finalize().into())
}

fn derive_code(shared_secret: &Key32, transcript: &Key32) -> Result<PairingCode> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, transcript).extract(shared_secret);
    let mut okm = [0u8; 4];
    prk.expand(&[CODE_CONTEXT], Len4)
        .and_then(|o| o.fill(&mut okm))
        .map_err(|_| Error::Crypto("HKDF failed"))?;
    // Bias from the modulo is ~2⁻¹², irrelevant for a human-checked code.
    Ok(PairingCode(u32::from_be_bytes(okm) % 1_000_000))
}

struct Len4;

impl hkdf::KeyType for Len4 {
    fn len(&self) -> usize {
        4
    }
}

fn confirm_payload(role: Role, transcript: &Key32) -> Vec<u8> {
    [CONFIRM_CONTEXT, &[role.tag()], transcript].concat()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::io::DuplexStream;

    use super::*;
    use crate::OsType;

    /// What a test confirmer was shown.
    type Seen = Arc<Mutex<Option<(DeviceInfo, PairingCode)>>>;

    struct Device {
        identity: Identity,
        info: DeviceInfo,
        trust: TrustStore,
    }

    impl Device {
        fn new(name: &str, os: OsType) -> Self {
            let identity = Identity::generate().unwrap();
            let info = identity.device_info(name, os);
            Self {
                identity,
                info,
                trust: TrustStore::in_memory(),
            }
        }
    }

    /// A confirmer that records the code it was shown and answers `accept`.
    fn recorder(accept: bool) -> (impl PinConfirmer, Seen) {
        let seen = Arc::new(Mutex::new(None));
        let seen2 = seen.clone();
        let confirmer = move |peer: DeviceInfo, code: PairingCode| {
            *seen2.lock().unwrap() = Some((peer, code));
            async move { accept }
        };
        (confirmer, seen)
    }

    async fn run_pairing(
        initiator: &Device,
        responder: &Device,
        initiator_accepts: bool,
        responder_accepts: bool,
    ) -> (
        Result<PairingOutcome>,
        Result<PairingOutcome>,
        Option<(DeviceInfo, PairingCode)>,
        Option<(DeviceInfo, PairingCode)>,
    ) {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let (ci, seen_i) = recorder(initiator_accepts);
        let (cr, seen_r) = recorder(responder_accepts);
        let (ri, rr) = tokio::join!(
            pair_as_initiator(
                &mut a,
                &initiator.identity,
                &initiator.info,
                &initiator.trust,
                &ci
            ),
            pair_as_responder(
                &mut b,
                &responder.identity,
                &responder.info,
                &responder.trust,
                &cr
            ),
        );
        let si = seen_i.lock().unwrap().clone();
        let sr = seen_r.lock().unwrap().clone();
        (ri, rr, si, sr)
    }

    fn pairing_err<T: fmt::Debug>(r: Result<T>) -> PairingError {
        match r {
            Err(Error::Pairing(e)) => e,
            other => panic!("expected pairing error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn successful_pairing_trusts_both_sides() {
        let phone = Device::new("Pixel", OsType::Android);
        let laptop = Device::new("ThinkPad", OsType::Linux);

        let (ri, rr, seen_i, seen_r) = run_pairing(&phone, &laptop, true, true).await;
        let (out_i, out_r) = (ri.unwrap(), rr.unwrap());

        // Both users were shown the same code, and the right peer.
        let (peer_seen_by_i, code_i) = seen_i.unwrap();
        let (peer_seen_by_r, code_r) = seen_r.unwrap();
        assert_eq!(code_i, code_r);
        assert_eq!(peer_seen_by_i, laptop.info);
        assert_eq!(peer_seen_by_r, phone.info);
        assert_eq!(out_i.code, out_r.code);
        assert_eq!(out_i.peer, laptop.info);
        assert_eq!(out_r.peer, phone.info);

        assert_eq!(phone.trust.get(laptop.info.id).unwrap().info(), laptop.info);
        assert_eq!(laptop.trust.get(phone.info.id).unwrap().info(), phone.info);
    }

    #[tokio::test]
    async fn codes_differ_between_sessions() {
        let a = Device::new("a", OsType::Linux);
        let b = Device::new("b", OsType::Windows);
        let mut codes = std::collections::HashSet::new();
        for _ in 0..5 {
            let (ri, _, _, _) = run_pairing(&a, &b, true, true).await;
            codes.insert(ri.unwrap().code);
        }
        // Fresh ephemeral keys every time; 5 identical codes would be a 10⁻²⁴ fluke.
        assert!(codes.len() > 1);
    }

    #[tokio::test]
    async fn responder_rejection_fails_both_sides() {
        let a = Device::new("a", OsType::Android);
        let b = Device::new("b", OsType::MacOS);
        let (ri, rr, _, _) = run_pairing(&a, &b, true, false).await;
        assert_eq!(pairing_err(ri), PairingError::RejectedByPeer);
        assert_eq!(pairing_err(rr), PairingError::RejectedLocally);
        assert!(a.trust.list().is_empty());
        assert!(b.trust.list().is_empty());
    }

    #[tokio::test]
    async fn initiator_rejection_fails_both_sides() {
        let a = Device::new("a", OsType::Android);
        let b = Device::new("b", OsType::MacOS);
        let (ri, rr, _, _) = run_pairing(&a, &b, false, true).await;
        assert_eq!(pairing_err(ri), PairingError::RejectedLocally);
        assert_eq!(pairing_err(rr), PairingError::RejectedByPeer);
        assert!(a.trust.list().is_empty());
        assert!(b.trust.list().is_empty());
    }

    #[tokio::test]
    async fn man_in_the_middle_sees_different_codes() {
        // Mallory relays between Alice and Bob, running honest pairing with
        // each using her own identity. The codes Alice and Bob see differ,
        // and each sees Mallory's identity rather than the other's.
        let alice = Device::new("alice", OsType::Android);
        let bob = Device::new("bob", OsType::Linux);
        let mallory_a = Device::new("bob", OsType::Linux); // faces Alice
        let mallory_b = Device::new("alice", OsType::Android); // faces Bob

        let (mut alice_end, mut m1) = tokio::io::duplex(4096);
        let (mut m2, mut bob_end) = tokio::io::duplex(4096);
        let (ca, seen_a) = recorder(true);
        let (cb, seen_b) = recorder(true);
        let (cm, _) = recorder(true);

        let _ = tokio::join!(
            pair_as_initiator(
                &mut alice_end,
                &alice.identity,
                &alice.info,
                &alice.trust,
                &ca
            ),
            pair_as_responder(
                &mut m1,
                &mallory_a.identity,
                &mallory_a.info,
                &mallory_a.trust,
                &cm
            ),
            pair_as_initiator(
                &mut m2,
                &mallory_b.identity,
                &mallory_b.info,
                &mallory_b.trust,
                &cm
            ),
            pair_as_responder(&mut bob_end, &bob.identity, &bob.info, &bob.trust, &cb),
        );

        let (peer_a, code_a) = seen_a.lock().unwrap().clone().unwrap();
        let (peer_b, code_b) = seen_b.lock().unwrap().clone().unwrap();
        // Independent random codes: equal with probability 10⁻⁶.
        assert_ne!(code_a, code_b);
        assert_ne!(peer_a.public_key, bob.info.public_key);
        assert_ne!(peer_b.public_key, alice.info.public_key);
        assert_ne!(peer_a.fingerprint(), bob.info.fingerprint());
    }

    /// Drive the responder side by hand to test what an honest initiator does
    /// with a malicious responder.
    async fn malicious_responder(
        stream: &mut DuplexStream,
        claimed: &DeviceInfo,
        signer: &Identity,
    ) -> Result<()> {
        let request = match read_frame(stream).await? {
            PairingMessage::Request(r) => r,
            _ => unreachable!(),
        };
        let (secret, public) = ephemeral_keypair()?;
        let response = PairingResponse {
            version: PROTOCOL_VERSION,
            device: claimed.clone(),
            ephemeral_key: public,
            nonce: random_32()?,
        };
        write_frame(stream, &PairingMessage::Response(response.clone())).await?;
        let (eph, nonce) = match read_frame(stream).await? {
            PairingMessage::Reveal {
                ephemeral_key,
                nonce,
            } => (ephemeral_key, nonce),
            _ => unreachable!(),
        };
        let transcript = transcript(&request, &response, &eph, &nonce)?;
        let _ = agree(secret, &eph)?;
        let _: PairingMessage = read_frame(stream).await?; // initiator's Confirm
        let signature = signer.sign(&confirm_payload(Role::Responder, &transcript));
        write_frame(stream, &PairingMessage::Confirm { signature }).await?;
        Ok(())
    }

    #[tokio::test]
    async fn impersonation_without_private_key_is_rejected() {
        // Mallory claims to be Bob (Bob's DeviceInfo, valid id/key pair) but
        // can only sign with her own key. Even if the user confirms the code,
        // the initiator must refuse to trust her.
        let alice = Device::new("alice", OsType::Android);
        let bob = Device::new("bob", OsType::Linux);
        let mallory = Identity::generate().unwrap();

        let (mut a, mut m) = tokio::io::duplex(4096);
        let (ca, _) = recorder(true);
        let (ri, rm) = tokio::join!(
            pair_as_initiator(&mut a, &alice.identity, &alice.info, &alice.trust, &ca),
            malicious_responder(&mut m, &bob.info, &mallory),
        );
        rm.unwrap();
        assert_eq!(pairing_err(ri), PairingError::BadSignature);
        assert!(alice.trust.list().is_empty());
    }

    #[tokio::test]
    async fn reveal_not_matching_commitment_is_rejected() {
        let mallory = Device::new("mallory", OsType::Windows);
        let bob = Device::new("bob", OsType::Linux);
        let (mut m, mut b) = tokio::io::duplex(4096);
        let (cb, seen_b) = recorder(true);

        let attacker = async {
            let (_, committed_key) = ephemeral_keypair()?;
            let nonce = random_32()?;
            let request = PairingRequest {
                version: PROTOCOL_VERSION,
                device: mallory.info.clone(),
                commitment: commitment(&committed_key, &nonce),
            };
            write_frame(&mut m, &PairingMessage::Request(request)).await?;
            let _: PairingMessage = read_frame(&mut m).await?;
            // Reveal a *different* key than the one committed to.
            let (_, other_key) = ephemeral_keypair()?;
            write_frame(
                &mut m,
                &PairingMessage::Reveal {
                    ephemeral_key: other_key,
                    nonce,
                },
            )
            .await?;
            read_frame::<_, PairingMessage>(&mut m).await
        };
        let (rm, rb) = tokio::join!(
            attacker,
            pair_as_responder(&mut b, &bob.identity, &bob.info, &bob.trust, &cb),
        );

        assert_eq!(pairing_err(rb), PairingError::CommitmentMismatch);
        assert_eq!(rm.unwrap(), PairingMessage::Reject);
        assert!(seen_b.lock().unwrap().is_none(), "user must not be asked");
        assert!(bob.trust.list().is_empty());
    }

    async fn send_request_expect_error(request: PairingRequest) -> PairingError {
        let bob = Device::new("bob", OsType::Linux);
        let (mut m, mut b) = tokio::io::duplex(4096);
        let (cb, _) = recorder(true);
        let sender = async {
            write_frame(&mut m, &PairingMessage::Request(request)).await?;
            read_frame::<_, PairingMessage>(&mut m).await
        };
        let (reply, rb) = tokio::join!(
            sender,
            pair_as_responder(&mut b, &bob.identity, &bob.info, &bob.trust, &cb),
        );
        assert_eq!(reply.unwrap(), PairingMessage::Reject);
        pairing_err(rb)
    }

    #[tokio::test]
    async fn spoofed_device_id_is_rejected() {
        let mut info = Device::new("mallory", OsType::Linux).info;
        info.id = Device::new("victim", OsType::Linux).info.id;
        let err = send_request_expect_error(PairingRequest {
            version: PROTOCOL_VERSION,
            device: info,
            commitment: [0; 32],
        })
        .await;
        assert!(matches!(err, PairingError::InvalidPeer(_)));
    }

    #[tokio::test]
    async fn unsupported_version_is_rejected() {
        let err = send_request_expect_error(PairingRequest {
            version: PROTOCOL_VERSION + 1,
            device: Device::new("future", OsType::Linux).info,
            commitment: [0; 32],
        })
        .await;
        assert_eq!(err, PairingError::UnsupportedVersion(PROTOCOL_VERSION + 1));
    }

    #[tokio::test]
    async fn unexpected_first_message_is_rejected() {
        let bob = Device::new("bob", OsType::Linux);
        let (mut m, mut b) = tokio::io::duplex(4096);
        let (cb, _) = recorder(true);
        write_frame(&mut m, &PairingMessage::Confirm { signature: vec![] })
            .await
            .unwrap();
        let err = pair_as_responder(&mut b, &bob.identity, &bob.info, &bob.trust, &cb).await;
        assert_eq!(pairing_err(err), PairingError::UnexpectedMessage);
    }

    #[tokio::test]
    async fn mismatched_local_info_is_refused() {
        let a = Device::new("a", OsType::Linux);
        let other = Device::new("b", OsType::Linux);
        let (mut s, _peer) = tokio::io::duplex(64);
        let (c, _) = recorder(true);
        let err = pair_as_initiator(&mut s, &a.identity, &other.info, &a.trust, &c).await;
        assert!(matches!(err, Err(Error::InvalidDeviceInfo(_))));
    }

    #[test]
    fn code_is_six_digits_and_deterministic() {
        assert_eq!(PairingCode(42).to_string(), "000042");
        assert_eq!(PairingCode(987_654).to_string(), "987654");

        let shared = [7u8; 32];
        let t = [9u8; 32];
        let c1 = derive_code(&shared, &t).unwrap();
        assert_eq!(c1, derive_code(&shared, &t).unwrap());
        assert!(c1.value() < 1_000_000);
        assert_eq!(c1.to_string().len(), 6);
    }

    #[test]
    fn code_depends_on_secret_and_transcript() {
        let base = derive_code(&[1; 32], &[2; 32]).unwrap();
        let mut differing = 0;
        for i in 0..32u8 {
            let mut s = [1u8; 32];
            s[usize::from(i)] ^= 0x80;
            let mut t = [2u8; 32];
            t[usize::from(i)] ^= 0x80;
            differing += usize::from(derive_code(&s, &[2; 32]).unwrap() != base);
            differing += usize::from(derive_code(&[1; 32], &t).unwrap() != base);
        }
        assert_eq!(differing, 64);
    }

    #[test]
    fn confirm_payload_is_role_bound() {
        let t = [3u8; 32];
        assert_ne!(
            confirm_payload(Role::Initiator, &t),
            confirm_payload(Role::Responder, &t)
        );
    }

    #[test]
    fn pairing_messages_round_trip() {
        let info = Device::new("a", OsType::MacOS).info;
        let msgs = [
            PairingMessage::Request(PairingRequest {
                version: 1,
                device: info.clone(),
                commitment: [1; 32],
            }),
            PairingMessage::Response(PairingResponse {
                version: 1,
                device: info,
                ephemeral_key: [2; 32],
                nonce: [3; 32],
            }),
            PairingMessage::Reveal {
                ephemeral_key: [4; 32],
                nonce: [5; 32],
            },
            PairingMessage::Confirm {
                signature: vec![6; 64],
            },
            PairingMessage::Reject,
        ];
        for m in msgs {
            let bytes = encode(&m).unwrap();
            assert_eq!(
                crate::protocol::decode::<PairingMessage>(&bytes).unwrap(),
                m
            );
        }
    }
}
