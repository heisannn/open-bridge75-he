/// Failure to parse or validate a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("unknown key name: {0} (run `bridge75 keys` to list them)")]
    UnknownKey(String),
    #[error("unknown key code: {0}")]
    UnknownUsage(String),
    #[error("cannot parse key action: {0}")]
    InvalidAction(String),
    #[error("{0}")]
    OutOfRange(String),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
