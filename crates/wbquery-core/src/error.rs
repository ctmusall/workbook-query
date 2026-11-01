//! The engine's error: a message meant for people (the same wording the Python version used).

/// What went wrong, in words to show the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A problem with the query, its values or the input data.
    #[error("{0}")]
    Value(String),
    /// A file that doesn't exist.
    #[error("{0}")]
    NotFound(String),
    /// A source alias the query doesn't have (`--source NAME=...`).
    #[error("{0}")]
    Key(String),
}

impl Error {
    pub fn msg(text: impl Into<String>) -> Error {
        Error::Value(text.into())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
