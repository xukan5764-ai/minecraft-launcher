use serde::{Serialize, Serializer};

/// Error type returned by Tauri commands; serialized to the frontend as a plain message.
#[derive(Debug)]
pub struct Error(anyhow::Error);

pub type Result<T> = std::result::Result<T, Error>;

impl<E: Into<anyhow::Error>> From<E> for Error {
    fn from(err: E) -> Self {
        Error(err.into())
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        // `{:#}` includes the context chain, e.g. "下载失败: connection reset".
        serializer.serialize_str(&format!("{:#}", self.0))
    }
}
