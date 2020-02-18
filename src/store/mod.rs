use std::error::Error;
use std::ffi::CString;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::docker::Reference;
use crate::oci::{GoString, Pull};
use crate::server::Image;

#[derive(Clone, Debug, Default)]
pub struct ImageStore {
    root_dir: PathBuf,
    images: Arc<RwLock<Vec<Image>>>,
}

/// An error which can be returned when there was an error
#[derive(Debug)]
pub enum ImageStoreError {
    CannotPullModule,
    InvalidPullPath,
    InvalidReference,
    LockNotAcquired,
    NotFound,
}

impl fmt::Display for ImageStoreError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ImageStoreError::CannotPullModule => f.write_str("cannot pull module"),
            ImageStoreError::InvalidPullPath => f.write_str("invalid pull path"),
            ImageStoreError::InvalidReference => f.write_str("invalid reference"),
            ImageStoreError::LockNotAcquired => f.write_str("cannot acquire lock on store"),
            ImageStoreError::NotFound => f.write_str("image not found"),
        }
    }
}

impl Error for ImageStoreError {
    fn description(&self) -> &str {
        match *self {
            ImageStoreError::CannotPullModule => "Cannot pull module",
            ImageStoreError::InvalidPullPath => "Invalid pull path",
            ImageStoreError::InvalidReference => "Invalid reference",
            ImageStoreError::LockNotAcquired => "Cannot acquire lock on store",
            ImageStoreError::NotFound => "Image not found",
        }
    }
}

impl ImageStore {
    pub fn new(root_dir: PathBuf) -> Self {
        // TODO(bacongobbler): populate `images` using `root_dir`
        ImageStore {
            root_dir: root_dir,
            images: Arc::new(RwLock::new(vec![])),
        }
    }

    pub fn add(&mut self, image: Image) -> Result<(), ImageStoreError> {
        let mut images = match self.images.write() {
            Ok(images) => images,
            Err(_) => {
                return Err(ImageStoreError::LockNotAcquired)
            }
        };
        images.push(image);
        Ok(())
    }

    pub fn list(&self) -> Vec<Image> {
        let images = self.images.read().unwrap();
        (*images.clone()).to_vec()
    }

    pub fn remove(&mut self, key: String) -> Result<Image, ImageStoreError> {
        let mut images = self.images.write().or(Err(ImageStoreError::LockNotAcquired))?;
        let i = images.iter().position(|i| i.id == key).ok_or_else(|| ImageStoreError::NotFound)?;
        Ok(images.remove(i))
    }

    pub fn pull(&mut self, reference: Reference) -> Result<(), ImageStoreError> {
        let pull_path = self.pull_path(reference);
        std::fs::create_dir_all(&pull_path).expect("could not create pull path");
        pull_wasm(reference, self.pull_file_path(reference))?;
        // TODO(bacongobbler): fetch image information from the module
        let i = Image {
            id: String::from(reference.whole),
            repo_digests: vec![],
            repo_tags: vec![],
            size: 0,
            uid: None,
            username: "".to_owned(),
        };
        self.add(i)
    }

    pub(crate) fn root_dir(&self) -> &PathBuf {
        &self.root_dir
    }

    pub(crate) fn used_bytes(&self) -> u64 {
        let images = self.images.read().unwrap();
        images.iter().map(|i| i.size).sum()
    }

    pub(crate) fn used_inodes(&self) -> u64 {
        let images = self.images.read().unwrap();
        images.len() as u64
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

fn pull_wasm(reference: Reference, fp: PathBuf) -> Result<(), ImageStoreError> {
    println!("pulling {} into {}", reference.whole, fp.to_str().unwrap());
    let c_ref = CString::new(reference.whole).or(Err(ImageStoreError::InvalidReference))?;
    let c_file = CString::new(fp.to_str().unwrap()).or(Err(ImageStoreError::InvalidPullPath))?;

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
        _ => Err(ImageStoreError::CannotPullModule),
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
