use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LoadedRom {
    path: PathBuf,
    data: Vec<u8>,
}

impl LoadedRom {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn into_parts(self) -> (PathBuf, Vec<u8>) {
        (self.path, self.data)
    }
}

pub fn load_rom_path(path: &Path) -> io::Result<LoadedRom> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut data = Vec::new();
    reader.read_to_end(&mut data)?;
    Ok(LoadedRom {
        path: path.to_path_buf(),
        data,
    })
}
