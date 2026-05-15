pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Bytes that do not follow the layout of the format being parsed. `offset` is relative to
    /// the start of that format's data.
    #[error("malformed {format} at {offset:#x}: {reason}")]
    Malformed {
        format: &'static str,
        offset: u64,
        reason: &'static str,
    },
    /// A file, archive member, record, or asset that does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// Input romkit recognizes but cannot handle, such as a game without an extractor.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// A failure of the embedding application: its byte source or its output.
    #[error("host error: {0}")]
    Host(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub(crate) fn malformed(format: &'static str, offset: u64, reason: &'static str) -> Self {
        Self::Malformed {
            format,
            offset,
            reason,
        }
    }
}

/// Lets sources and outputs backed by `std::io` use `?`.
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Host(error.to_string())
    }
}
