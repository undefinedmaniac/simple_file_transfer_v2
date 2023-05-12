use std::{ffi::OsString, path::Path};

use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use time::OffsetDateTime;

pub mod mapped_fs;
pub mod browser;

/// Represents a file/directory in a file system
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct FSElement {
    pub name: OsString,
    pub created: Option<OffsetDateTime>,
    pub modified: Option<OffsetDateTime>,
    pub size: u64,
    /// True if the element is a file, false if it is a directory
    pub is_file: bool
}

#[async_trait]
pub trait FS {
    type Error: std::error::Error;

    /// List the elements at a specified path within the file system
    async fn list<P: AsRef<Path> + Send + Sync>(&self, path: P) -> Result<Vec<FSElement>, Self::Error>;
}
