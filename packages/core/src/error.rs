use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("frame is {0} bytes, exceeding the maximum allowed size")]
    FrameTooLarge(usize),

    #[error("incomplete frame: need {needed} bytes, have {available}")]
    IncompleteFrame { needed: usize, available: usize },

    #[error("malformed message: {0}")]
    Malformed(#[from] serde_json::Error),

    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u16),
}
