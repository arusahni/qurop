#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("An IO error has occurred: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },
}
