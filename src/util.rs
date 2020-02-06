use std::path::PathBuf;

pub fn ensure_root_dir(dir: &PathBuf) -> Result<(), failure::Error> {
    std::fs::create_dir_all(dir)?;
    Ok(())
}
