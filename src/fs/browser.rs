use std::{
    collections::{HashMap, hash_map::Entry},
    path::{Path, PathBuf}, cmp::Ordering,
};

use rand::{distributions::Uniform, prelude::Distribution, rngs::SmallRng, SeedableRng};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::FSElement;

use super::FS;

#[derive(Deserialize, Serialize)]
pub enum Request {
    // Create a new Cursor and return it's ID
    Create,
    // Destroy a Cursor, freeing it's resources
    Destroy { id: u16 },

    // Read the file data from the Cursor's current position
    Read { id: u16 },

    // Get the current location (path) of the Cursor
    GetLocation { id: u16 },
    // Move the cursor to a new location
    Move { id: u16, path: PathBuf },
}

#[derive(Deserialize, Serialize)]
pub enum Response {
    // On success, returns the ID of the cursor
    Create(Result<u16, CursorError>),

    // The Ok(()) value means the cursor was destroyed successfully
    Destroy(Result<(), CursorError>),

    // Returns a list of the file system elements that were read
    Read(Result<Vec<FSElement>, CursorError>),

    // Only fails if the cursor ID is wrong
    GetLocation(Result<PathBuf, CursorError>),
    // Only fails if the cursor ID is wrong
    Move(Result<(), CursorError>)
}

#[derive(Error, Debug, Deserialize, Serialize)]
pub enum CursorError {
    #[error("A new cursor cannot be created, since the limit of {limit} cursors has already been reached")]
    CursorLimitReached { limit: u16 },

    #[error("The specified cursor does not exist")]
    UnknownCursor,

    #[error("The path {path} is not readable")]
    ReadError { path: PathBuf },
}

struct Cursor {
    path: PathBuf,
    state: Option<Vec<FSElement>>
}

pub struct Browser<F> {
    cursors: HashMap<u16, Cursor>,
    cursor_limit: u16,

    cursor_id_rng: SmallRng,
    cursor_id_uniform: Uniform<u16>,

    fs: F
}

impl<F: FS> Browser<F> {
    pub fn new(cursor_limit: u16, fs: F) -> Self {
        Browser {
            cursors: HashMap::new(),
            cursor_limit,
            cursor_id_rng: SmallRng::from_entropy(),
            cursor_id_uniform: Uniform::new_inclusive(0, u16::MAX),
            fs,
        }
    }

    pub fn create_cursor(&mut self) -> Result<u16, CursorError> {
        if self.cursors.len() >= self.cursor_limit.into() {
            return Err(CursorError::CursorLimitReached {
                limit: self.cursor_limit,
            });
        }

        loop {
            let id = self.cursor_id_uniform.sample(&mut self.cursor_id_rng);

            if let Entry::Vacant(entry) = self.cursors.entry(id) {
                entry.insert(
                    Cursor {
                        path: PathBuf::new(),
                        state: None
                    },
                );
                return Ok(id);
            }
        }
    }

    pub fn destroy_cursor(&mut self, id: u16) -> Result<(), CursorError> {
        self.cursors
            .remove(&id)
            .map(|_| ())
            .ok_or(CursorError::UnknownCursor)
    }

    pub async fn read_cursor(&mut self, id: u16) -> Result<&Vec<FSElement>, CursorError> {
        let cursor = get_cursor_mut(&mut self.cursors, id)?;
        self.fs
            .list(&cursor.path)
            .await
            .map_err(|_| CursorError::ReadError { path: cursor.path.clone() })
            .map(|mut elements| {
                elements.sort_unstable_by(cmp_fs_elements);
                cursor.state = Some(elements);
                cursor.state.as_ref().unwrap()
            })
    }

    pub fn get_location_cursor(&self, id: u16) -> Result<&Path, CursorError> {
        Ok(&get_cursor(&self.cursors, id)?.path)
    }

    pub fn move_cursor<P: AsRef<Path>>(&mut self, id: u16, path: P) -> Result<(), CursorError> {
        let cursor = get_cursor_mut(&mut self.cursors, id)?;
        if &cursor.path != path.as_ref() {
            cursor.path = path.as_ref().to_owned();
            cursor.state = None;
        }
        Ok(())
    }

    pub async fn process(&mut self, request: Request) -> Response {
        match request {
            Request::Create => Response::Create(self.create_cursor()),
            Request::Destroy { id } => Response::Destroy(self.destroy_cursor(id)),
            Request::Read { id } => Response::Read(self.read_cursor(id).await.cloned()),
            Request::GetLocation { id } => Response::GetLocation(self.get_location_cursor(id)
                .map(ToOwned::to_owned)),
            Request::Move { id, path } => Response::Move(self.move_cursor(id, path)),
        }
    }
}

fn cmp_fs_elements(element1: &FSElement, element2: &FSElement) -> Ordering {
    element1.name.cmp(&element2.name)
}

fn get_cursor(cursors: &HashMap<u16, Cursor>, id: u16) -> Result<&Cursor, CursorError> {
    cursors
        .get(&id)
        .ok_or(CursorError::UnknownCursor)
}

fn get_cursor_mut(cursors: &mut HashMap<u16, Cursor>, id: u16) -> Result<&mut Cursor, CursorError> {
    cursors
        .get_mut(&id)
        .ok_or(CursorError::UnknownCursor)
}
