#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("An IO error has occurred: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },
    #[error("The window is not found")]
    WindowNotFound,
    #[error("Configuration error: {source}")]
    Config {
        #[from]
        source: confy::ConfyError,
    },
}
