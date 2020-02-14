use std::fs::File;
use std::io::BufReader;

/// Result describes a Runtime result that may return a failure::Error if things go wrong.
pub type Result<T> = std::result::Result<T, failure::Error>;

pub trait Runtime {
    fn run(&self) -> Result<()>;
    fn output(&self) -> Result<(BufReader<File>, BufReader<File>)>;
}
