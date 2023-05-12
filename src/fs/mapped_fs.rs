use std::{path::{PathBuf, Path, Component}, collections::{HashMap, hash_map::Entry}, ffi::{OsString, OsStr}, sync::{RwLock, Arc}};
use std::{time::SystemTime, io};

use anyhow::Context;
use async_trait::async_trait;
use thiserror::Error;
use time::{OffsetDateTime, UtcOffset};

use super::{FSElement, FS};

/// Convert a SystemTime into a OffsetDateTime with the local offset
fn convert_time(time: SystemTime) -> Result<OffsetDateTime, anyhow::Error>
{
    let duration = time.duration_since(SystemTime::UNIX_EPOCH)
        .context("Time value too small for conversion")?;
    let converted: i64 = duration.as_secs()
        .try_into()
        .context("Time value too large for conversion")?;

    // If we are capable of obtaining the system offset, use it to adjust the timestamp
    let mut timestamp = OffsetDateTime::from_unix_timestamp(converted)?;
    if let Ok(offset) = UtcOffset::current_local_offset() {
        timestamp = timestamp.to_offset(offset);
    }

    Ok(timestamp)
    // Ok(OffsetDateTime::from_unix_timestamp(converted)?)
}

/// Obtain a file system element. Looks up metadata from the real file system and may fail
async fn get_element<S: AsRef<OsStr>, P: AsRef<Path>>(name: S, path: P) -> Result<FSElement, io::Error> {
    let metadata = tokio::fs::metadata(path).await?;
    
    let created = metadata
        .created()
        .map_err(anyhow::Error::new)
        .and_then(convert_time)
        .ok();

    let modified = metadata
        .modified()
        .map_err(anyhow::Error::new)
        .and_then(convert_time)
        .ok();

    let element = FSElement {
        name: name.as_ref().to_owned(),
        created,
        modified,
        size: metadata.len(),
        is_file: metadata.is_file(),
    };

    Ok(element)
}

#[derive(Error, Debug)]
pub enum MappedFSError {
    #[error("The path {0} does not exist in the mapped file system")]
    PathNotFound(PathBuf, #[source] anyhow::Error),

    #[error("The path {0} is not absolute. Only absolute paths can be added")]
    PathNotAbsolute(PathBuf)
}

enum ParsedPath {
    Root,
    Extended { root_element: OsString, extension: PathBuf }
}

fn parse_path<P: AsRef<Path>>(path: P) -> Result<ParsedPath, MappedFSError> {
    let path_not_found_err =
        |err| MappedFSError::PathNotFound(path.as_ref().to_owned(), err);

    // Ensure the path is limited to the designated file system area
    let mut components = path.as_ref().components();
    let mut first_component = true;
    let mut parsed_path = ParsedPath::Root;
    while let Some(component) = components.next() {
        match component {
            Component::Prefix(..) | Component::CurDir if first_component => (), 
            Component::RootDir => (),
            Component::Normal(_name) if matches!(parsed_path, ParsedPath::Root) => {
                parsed_path = ParsedPath::Extended {
                    root_element: _name.to_owned(), 
                    extension: components.as_path().to_owned()
                };
            }
            Component::Normal(..) => (),
            _ => return Err(path_not_found_err(anyhow::anyhow!("The path contains illegal components such as '.' or '..'")))
        }
        first_component = false;
    };

    Ok(parsed_path)
}

#[derive(Clone)]
pub struct MappedFS {
    map: Arc<RwLock<HashMap<OsString, PathBuf>>>
}

impl MappedFS {
    pub fn new() -> Self {
        MappedFS { map: Arc::new(RwLock::new(HashMap::new())) }
    }

    /// Add a new file or directory to the mapped filesystem. Does nothing if the element has already been
    /// added previously. If two different elements with the same name are added, a number will be appended
    /// to the name of the more recent element. For example, if two files named 'test.txt' are added, the
    /// name of the second file within the virtual filesystem will be 'test.txt (1)'
    pub fn add<P: AsRef<Path>>(&mut self, path: P) -> Result<(), MappedFSError> {
        let path = path.as_ref();

        if !path.is_absolute() {
            return Err(MappedFSError::PathNotAbsolute(path.to_owned()));
        }

        // let bad_path_err = 
        //     |err: anyhow::Error| MappedFSError::InvalidMapping(path.as_ref().to_owned(), err);

        // let absolute_path = fs::canonicalize(&path)
        //     .map_err(anyhow::Error::from)
        //     .map_err(bad_path_err)?;

        // if !absolute_path.try_exists()
        //     .map_err(anyhow::Error::from)
        //     .map_err(bad_path_err)? 
        // {
        //     return Err(bad_path_err(anyhow::anyhow!("Broken symbolic link in path")));
        // }
        
        // This should never fail, since the previous steps verify that the path is valid
        let name_in_path = path
            .file_name()
            .unwrap();

        let mut number: u32 = 0;
        let mut map = self.map.write().unwrap();
        loop {
            let name = if number == 0 {
                name_in_path.to_owned()
            } else {
                let number_str = OsString::from(format!(" ({number})"));

                let mut name_with_number = OsString::with_capacity(name_in_path.len() + number_str.len());
                name_with_number.push(name_in_path);
                name_with_number.push(number_str);

                name_with_number
            };

            match map.entry(name) {
                // The file/directory is already in the VFS, so nothing needs to be done
                Entry::Occupied(entry) if entry.get() == path => break,

                // An existing file/directory has the same name, so add a number to the end
                Entry::Occupied(_) => {
                    assert!(number != u32::MAX);
                    number += 1;
                }

                // The name is unique and this is a new file/directory, we can insert!
                Entry::Vacant(entry) => {
                    entry.insert(path.to_owned());
                    break;
                }
            }
        }

        Ok(())
    }

    /// Returns a list of the currently registered paths
    pub fn registered(&self) -> Vec<PathBuf> {
        self.map.read().unwrap().values().cloned().collect()
    }

    /// Remove a path from the mapped FS
    pub fn remove<P: AsRef<Path>>(&mut self, path: P) {
        self.map.write().unwrap().retain(|_, _path| _path != path.as_ref());
    }

    /// Unmap a mapped path to obtain the path within the real file system
    pub fn unmap<P: AsRef<Path>>(&self, path: P) -> Result<PathBuf, MappedFSError> {
        let path_not_found_err =
            |err| MappedFSError::PathNotFound(path.as_ref().to_owned(), err);

        match parse_path(&path)? {
            ParsedPath::Extended { root_element, extension } => {
                let map = self.map.read().unwrap();
                let absolute_path = map.get(&root_element)
                    .ok_or_else(|| path_not_found_err(anyhow::anyhow!("The root element of the path does not exist")))?;

                Ok(absolute_path.join(extension))
            }
            ParsedPath::Root => Err(path_not_found_err(anyhow::anyhow!("A path to the root of the mapped file system cannot be unmapped"))),
        }
    }

    /// List the FSElements at the specified path within the mapped FS
    pub async fn list<P: AsRef<Path>>(&self, path: P) -> Result<Vec<FSElement>, MappedFSError> {
        let path_not_found_err =
            |err| MappedFSError::PathNotFound(path.as_ref().to_owned(), err);

        let iter = match parse_path(&path)? {
            ParsedPath::Root => {
                // This is a path to the root of the mapped FS
                let contents: Vec<(OsString, PathBuf)> = self.map
                    .read()
                    .unwrap()
                    .iter()
                    .map(|(name, path)| (name.to_owned(), path.to_owned()))
                    .collect();

                contents
                    .into_iter()
                    .map(|(name, absolute_path)| tokio::spawn(get_element(name, absolute_path)))
                    .collect()
            }
            ParsedPath::Extended { root_element, extension } => {
                // This path goes deeper into the mapped FS
                let path = {
                    let map = self.map.read().unwrap();
                    let absolute_path = map.get(&root_element)
                        .ok_or_else(|| path_not_found_err(anyhow::anyhow!("The root element of the path does not exist")))?;

                    absolute_path.join(extension)
                };

                let mut read_dir = tokio::fs::read_dir(path)
                    .await
                    .map_err(anyhow::Error::from)
                    .map_err(path_not_found_err)?;

                let mut futures = vec![];
                while let Some(entry) = read_dir.next_entry()
                    .await
                    .map_err(anyhow::Error::from)
                    .map_err(path_not_found_err)?
                {
                    futures.push(tokio::spawn(get_element(entry.file_name(), entry.path())));
                }

                futures
            }
        };

        let mut output = vec![];
        for handle in iter {
            if let Ok(elements) = handle.await.unwrap() {
                output.push(elements);
            }
        }
        Ok(output)
    }
}

#[async_trait]
impl FS for MappedFS {
    type Error = MappedFSError;

    async fn list<P: AsRef<Path> + Send + Sync>(&self, path: P) -> Result<Vec<FSElement>, MappedFSError> {
        self.list(path).await
    }
}
