use diffz_core::provider::ServiceError;
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("{0}")]
    Message(String),
    #[error("I/O operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("draft store operation failed: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("unexpected JSON document: {0}")]
    Json(#[from] serde_json::Error),
    #[error("patch cannot be loaded: {0}")]
    Patch(#[from] diffz_core::patch::PatchError),
}
pub type Result<T> = std::result::Result<T, AdapterError>;
impl From<String> for AdapterError {
    fn from(s: String) -> Self {
        Self::Message(s)
    }
}
impl From<&str> for AdapterError {
    fn from(s: &str) -> Self {
        s.to_string().into()
    }
}
impl From<AdapterError> for ServiceError {
    fn from(e: AdapterError) -> Self {
        e.to_string().into()
    }
}
