use std::fs::File;
use std::io;

pub fn sync(file: &File) -> io::Result<()> {
    file.sync_all()
}
