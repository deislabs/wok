use std::error::Error;
use std::ffi::CString;
use std::fmt;
use std::fs;
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
    CannotFetchModuleMetadata,
    CannotPullModule,
    InvalidPullPath,
    InvalidReference,
    LockNotAcquired,
    NotFound,
}

impl fmt::Display for ModuleStoreError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ModuleStoreError::CannotFetchModuleMetadata => {
                f.write_str("cannot fetch metadata from the module")
            }
            ModuleStoreError::CannotPullModule => f.write_str("cannot pull module"),
            ModuleStoreError::InvalidPullPath => f.write_str("invalid pull path"),
            ModuleStoreError::InvalidReference => f.write_str("invalid reference"),
            ModuleStoreError::LockNotAcquired => f.write_str("cannot acquire lock on store"),
            ModuleStoreError::NotFound => f.write_str("module not found"),
        }
    }
}

impl Error for ModuleStoreError {
    fn description(&self) -> &str {
        match *self {
            ModuleStoreError::CannotFetchModuleMetadata => "Cannot fetch metadata from the module",
            ModuleStoreError::CannotPullModule => "Cannot pull module",
            ModuleStoreError::InvalidPullPath => "Invalid pull path",
            ModuleStoreError::InvalidReference => "Invalid reference",
            ModuleStoreError::LockNotAcquired => "Cannot acquire lock on store",
            ModuleStoreError::NotFound => "Module not found",
        }
    }
}

impl ModuleStore {
    pub fn new(root_dir: PathBuf) -> Self {
        // TODO(bacongobbler): populate `modules` using `root_dir`
        ModuleStore {
            root_dir,
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

    pub fn list(&self) -> Result<Vec<Module>, ModuleStoreError> {
        let modules = self
            .modules
            .read()
            .or(Err(ModuleStoreError::LockNotAcquired))?;
        Ok(modules.clone())
    }

    pub fn remove(&mut self, key: String) -> Result<Module, ModuleStoreError> {
        let mut modules = self
            .modules
            .write()
            .or(Err(ModuleStoreError::LockNotAcquired))?;
        let i = modules
            .iter()
            .position(|i| i.id == key)
            .ok_or(ModuleStoreError::NotFound)?;
        Ok(modules.remove(i))
    }

    pub fn pull(&mut self, reference: &Reference) -> Result<(), ModuleStoreError> {
        let pull_path = self.pull_path(reference);
        std::fs::create_dir_all(&pull_path).or(Err(ModuleStoreError::CannotPullModule))?;
        pull_wasm(&reference, self.pull_file_path(&reference))?;
        let attrs = fs::metadata(self.pull_file_path(&reference))
            .or(Err(ModuleStoreError::CannotFetchModuleMetadata))?;
        // TODO(bacongobbler): fetch image information from the module
        let m = Module {
            id: reference.whole().to_owned(),
            repo_digests: vec![],
            repo_tags: vec![],
            size: attrs.len(),
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

    pub(crate) fn pull_path(&self, r: &Reference) -> PathBuf {
        self.root_dir
            .join(r.registry())
            .join(r.repository())
            .join(r.tag())
    }

    pub(crate) fn pull_file_path(&self, r: &Reference) -> PathBuf {
        self.pull_path(r).join("module.wasm")
    }
}

fn pull_wasm(reference: &Reference, fp: PathBuf) -> Result<(), ModuleStoreError> {
    let filepath = fp.to_str().ok_or(ModuleStoreError::InvalidPullPath)?;
    println!("pulling {} into {}", reference.whole(), filepath);
    let c_ref = CString::new(reference.whole()).or(Err(ModuleStoreError::InvalidReference))?;
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
    let r = Reference::try_from(module).expect("Failed to parse reference");
    pull_wasm(&r, PathBuf::from("target/pulled.wasm")).unwrap();
}

#[test]
fn test_module_store_used_bytes() {
    let mut s = ModuleStore {
        root_dir: PathBuf::from("/"),
        modules: Arc::new(RwLock::new(vec![])),
    };
    assert_eq!(0, s.used_bytes().expect("could not retrieve used_bytes"));

    let m = Module {
        id: "1".to_owned(),
        repo_digests: vec![],
        repo_tags: vec![],
        size: 1,
        uid: None,
        username: "".to_owned(),
    };
    s.add(m).expect("could not add module to store");
    assert_eq!(1, s.used_bytes().expect("could not retrieve used_bytes"));

    let m2 = Module {
        id: "2".to_owned(),
        repo_digests: vec![],
        repo_tags: vec![],
        size: 2,
        uid: None,
        username: "".to_owned(),
    };
    s.add(m2).expect("could not add module to store");
    assert_eq!(3, s.used_bytes().expect("could not retrieve used_bytes"));

    s.remove("1".to_owned()).expect("could not remove module");
    assert_eq!(2, s.used_bytes().expect("could not retrieve used_bytes"));
}
