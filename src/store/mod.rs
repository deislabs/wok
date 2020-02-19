use std::error::Error;
use std::ffi::CString;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::docker::Reference;
use crate::oci::{GoString, Pull};
use crate::server::Module;

#[derive(Clone, Debug, Default)]
pub struct ModuleStore {
    root_dir: PathBuf,
    modules: Arc<RwLock<Vec<Module>>>,
}

/// An error which can be returned when there was an error
#[derive(Debug)]
pub enum ModuleStoreError {
    CannotPullModule,
    InvalidPullPath,
    InvalidReference,
    LockNotAcquired,
    NotFound,
}

impl fmt::Display for ModuleStoreError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ModuleStoreError::CannotPullModule => f.write_str("cannot pull module"),
            ModuleStoreError::InvalidPullPath => f.write_str("invalid pull path"),
            ModuleStoreError::InvalidReference => f.write_str("invalid reference"),
            ModuleStoreError::LockNotAcquired => f.write_str("cannot acquire lock on store"),
            ModuleStoreError::NotFound => f.write_str("image not found"),
        }
    }
}

impl Error for ModuleStoreError {
    fn description(&self) -> &str {
        match *self {
            ModuleStoreError::CannotPullModule => "Cannot pull module",
            ModuleStoreError::InvalidPullPath => "Invalid pull path",
            ModuleStoreError::InvalidReference => "Invalid reference",
            ModuleStoreError::LockNotAcquired => "Cannot acquire lock on store",
            ModuleStoreError::NotFound => "Image not found",
        }
    }
}

impl ModuleStore {
    pub fn new(root_dir: PathBuf) -> Self {
        // TODO(bacongobbler): populate `images` using `root_dir`
        ModuleStore {
            root_dir: root_dir,
            modules: Arc::new(RwLock::new(vec![])),
        }
    }

    pub fn add(&mut self, module: Module) -> Result<(), ModuleStoreError> {
        let mut modules = self
            .modules
            .write()
            .or(Err(ModuleStoreError::LockNotAcquired))?;
        modules.push(module);
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Image>, ModuleStoreError> {
        let modules = self
            .modules
            .read()
            .or(Err(ModuleStoreError::LockNotAcquired))?;
        Ok(modules.clone())
    }

    pub fn remove(&mut self, key: String) -> Result<Image, ModuleStoreError> {
        let mut modules = self.modules.write().or(Err(ModuleStoreError::LockNotAcquired))?;
        let i = modules.iter().position(|i| i.id == key).ok_or(ModuleStoreError::NotFound)?;
        Ok(modules.remove(i))
    }

    pub fn pull(&mut self, reference: Reference) -> Result<(), ModuleStoreError> {
        let pull_path = self.pull_path(reference);
        std::fs::create_dir_all(&pull_path).expect("could not create pull path");
        pull_wasm(reference, self.pull_file_path(reference))?;
        // TODO(bacongobbler): fetch image information from the module
        let m = Module {
            id: String::from(reference.whole),
            repo_digests: vec![],
            repo_tags: vec![],
            size: 0,
            uid: None,
            username: "".to_owned(),
        };
        self.add(m)
    }

    pub(crate) fn root_dir(&self) -> &PathBuf {
        &self.root_dir
    }

    pub(crate) fn used_bytes(&self) -> Result<u64, ModuleStoreError> {
       let modules = self
            .modules
            .read()
            .or(Err(ModuleStoreError::LockNotAcquired))?;
       Ok(modules.iter().map(|i| i.size).sum())
        
    }

    pub(crate) fn used_inodes(&self) -> Result<u64, ModuleStoreError> {
        let modules = self
            .modules
            .read()
            .or(Err(ModuleStoreError::LockNotAcquired))?;
        Ok(modules.len() as u64)
    }

    pub(crate) fn pull_path(&self, image_ref: Reference) -> PathBuf {
        self.root_dir
            .join(image_ref.registry)
            .join(image_ref.repo)
            .join(image_ref.tag)
    }

    pub(crate) fn pull_file_path(&self, image_ref: Reference) -> PathBuf {
        self.pull_path(image_ref).join("module.wasm")
    }
}

fn pull_wasm(reference: Reference, fp: PathBuf) -> Result<(), ModuleStoreError> {
    let filepath = fp
        .to_str()
        .ok_or_else(|| ModuleStoreError::InvalidPullPath)?;
    println!("pulling {} into {}", reference.whole, filepath);
    let c_ref = CString::new(reference.whole).or(Err(ModuleStoreError::InvalidReference))?;
    let c_file = CString::new(filepath).or(Err(ModuleStoreError::InvalidPullPath))?;

    let go_str_ref = GoString {
        p: c_ref.as_ptr(),
        n: c_ref.as_bytes().len() as isize,
    };
    let go_str_file = GoString {
        p: c_file.as_ptr(),
        n: c_file.as_bytes().len() as isize,
    };

    let result = unsafe { Pull(go_str_ref, go_str_file) };
    match result {
        0 => Ok(()),
        _ => Err(ModuleStoreError::CannotPullModule),
    }
}

#[test]
fn test_pull_wasm() {
    use std::convert::TryFrom;

    // this is a public registry, so this test is both making sure the library is working,
    // as well as ensuring the registry is publicly accessible
    let module = "webassembly.azurecr.io/hello-wasm:v1".to_owned();
    let image_ref = Reference::try_from(&module).expect("Failed to parse image_ref");
    pull_wasm(image_ref, PathBuf::from("target/pulled.wasm")).unwrap();
}
