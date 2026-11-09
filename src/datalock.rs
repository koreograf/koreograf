use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

pub fn lock_data_directory(data_dir: &Path) -> anyhow::Result<File> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join(".koreograf.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(anyhow::anyhow!(
            "The data directory {} is already in use by a running Koreograf server.",
            data_dir.display()
        )),
        Err(TryLockError::Error(err)) => Err(anyhow::anyhow!(
            "The data directory {} cannot be locked: {err}",
            data_dir.display()
        )),
    }
}
