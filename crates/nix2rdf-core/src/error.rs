use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, #[source] source: std::io::Error },
    #[error("nix command failed: {cmd}\n{stderr}")]
    Nix { cmd: String, stderr: String },
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),
    #[error("rdf parse error: {0}")]
    Rdf(String),
    #[error("store error: {0}")]
    Store(String),
    #[error("pack error: {0}")]
    Pack(String),
    #[error("reasoning error: {0}")]
    Reason(String),
    #[error("sparql error: {0}")]
    Sparql(String),
    #[error("kubernetes error: {0}")]
    Kube(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io { path: path.into(), source }
    }
}

impl From<oxigraph::store::StorageError> for Error {
    fn from(e: oxigraph::store::StorageError) -> Self { Error::Store(e.to_string()) }
}
impl From<oxigraph::sparql::QueryEvaluationError> for Error {
    fn from(e: oxigraph::sparql::QueryEvaluationError) -> Self { Error::Sparql(e.to_string()) }
}
impl From<oxigraph::store::LoaderError> for Error {
    fn from(e: oxigraph::store::LoaderError) -> Self { Error::Store(e.to_string()) }
}
impl From<oxrdf::IriParseError> for Error {
    fn from(e: oxrdf::IriParseError) -> Self { Error::Rdf(e.to_string()) }
}
impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self { Error::Other(format!("{e:#}")) }
}
