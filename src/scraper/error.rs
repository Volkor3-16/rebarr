use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScraperError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Browser error: {0}")]
    Browser(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Script error: {0}")]
    Script(String),

    #[error("No results found")]
    NotFound,

    #[error("Provider does not support this operation")]
    Unsupported,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML error in provider config: {0}")]
    Config(#[from] serde_yaml::Error),
}
