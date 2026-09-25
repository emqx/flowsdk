// SPDX-License-Identifier: MPL-2.0
//! Example JSON store. The caller must ensure a single writer per key.

use flowsdk::mqtt_client::{ClientSessionState, ClientSessionStore};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

pub struct FileSessionStore {
    directory: PathBuf,
}

impl FileSessionStore {
    pub fn new(directory: impl AsRef<Path>) -> io::Result<Self> {
        fs::create_dir_all(directory.as_ref())?;
        Ok(Self {
            directory: directory.as_ref().to_owned(),
        })
    }

    fn path(&self, key: &str) -> io::Result<PathBuf> {
        if key.is_empty() || key.len() > 100 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Key must contain 1-100 bytes",
            ));
        }
        Ok(self.directory.join(format!("{}.json", hex::encode(key))))
    }

    fn save(&self, key: &str, state: &ClientSessionState, create: bool) -> io::Result<()> {
        let path = self.path(key)?;
        if !create {
            fs::metadata(&path)?;
        }
        // Use the same filesystem so publishing the completed file is atomic.
        let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)?;
        serde_json::to_writer(temporary.as_file_mut(), state)?;
        temporary.as_file().sync_all()?;
        if create {
            temporary.persist_noclobber(path).map_err(|e| e.error)?;
        } else {
            temporary.persist(path).map_err(|e| e.error)?;
        }
        self.sync_directory()
    }

    fn sync_directory(&self) -> io::Result<()> {
        // Unix needs directory fsync to persist rename/unlink metadata. Other
        // platforms may require stronger backend-specific durability measures.
        #[cfg(unix)]
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}

impl ClientSessionStore for FileSessionStore {
    type Error = io::Error;

    fn create(&mut self, key: &str, state: &ClientSessionState) -> io::Result<()> {
        self.save(key, state, true)
    }

    fn resume(&mut self, key: &str) -> io::Result<Option<ClientSessionState>> {
        match File::open(self.path(key)?) {
            Ok(file) => Ok(Some(serde_json::from_reader(file)?)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn update(&mut self, key: &str, state: &ClientSessionState) -> io::Result<()> {
        self.save(key, state, false)
    }

    fn delete(&mut self, key: &str) -> io::Result<()> {
        match fs::remove_file(self.path(key)?) {
            Ok(()) => self.sync_directory(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}
