use crate::DeviceId;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to encode message: {0}")]
    Encode(#[from] bincode::error::EncodeError),

    #[error("failed to decode message: {0}")]
    Decode(#[from] bincode::error::DecodeError),

    #[error("frame of {0} bytes exceeds the maximum frame size")]
    FrameTooLarge(usize),

    #[error("{0} trailing bytes after message")]
    TrailingBytes(usize),

    #[error("connection closed by peer")]
    ConnectionClosed,

    #[error("timed out")]
    Timeout,

    #[error("invalid device info: {0}")]
    InvalidDeviceInfo(&'static str),

    #[error("pairing failed: {0}")]
    Pairing(#[from] PairingError),

    #[error("TLS error: {0}")]
    Tls(#[from] rustls::Error),

    #[error("mDNS error: {0}")]
    Mdns(#[from] mdns_sd::Error),

    #[error("key store error: {0}")]
    KeyStore(String),

    #[error("cryptographic failure: {0}")]
    Crypto(&'static str),

    #[error("trust store error: {0}")]
    TrustStore(#[from] serde_json::Error),

    #[error("device {0} is not paired")]
    NotPaired(DeviceId),

    #[error("device {0} is not connected")]
    NotConnected(DeviceId),

    #[error("peer is not a trusted device")]
    UntrustedPeer,

    #[error("protocol violation: {0}")]
    Protocol(&'static str),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PairingError {
    #[error("the other device rejected the pairing code")]
    RejectedByPeer,

    #[error("the pairing code was rejected on this device")]
    RejectedLocally,

    #[error("peer's revealed key does not match its commitment")]
    CommitmentMismatch,

    #[error("peer's confirmation signature is invalid")]
    BadSignature,

    #[error("unexpected message during pairing")]
    UnexpectedMessage,

    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u16),

    #[error("peer sent invalid device info: {0}")]
    InvalidPeer(&'static str),

    #[error("incoming pairing requests are disabled")]
    Disabled,
}
