use std::ffi::CString;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::oci::{GoString, Pull};
use crate::docker::ImageReference;
use crate::server::Image;

#[derive(Clone, Debug, Default)]
pub struct ImageStore {
    root_dir: PathBuf,
    images: Arc<RwLock<Vec<Image>>>,
}

/// An error which can be returned when there was an error
#[derive(Debug)]
pub struct ImageStoreErr {
    details: String,
}

impl ImageStoreErr {
    fn new(msg: &str) -> ImageStoreErr {
        ImageStoreErr {
            details: msg.to_string(),
        }
    }
}

impl fmt::Display for ImageStoreErr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.details)
    }
}

impl std::error::Error for ImageStoreErr {
    fn cause(&self) -> Option<&dyn std::error::Error> {
        Some(self)
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

    pub fn add(&mut self, image: Image) -> Result<(), ImageStoreErr> {
        let mut images = match self.images.write() {
            Ok(images) => images,
            Err(e) => {
                return Err(ImageStoreErr::new(&format!(
                    "Could not acquire store lock: {}",
                    e
                )))
            }
        };
        images.push(image);
        Ok(())
    }

    pub fn list(&self) -> Vec<Image> {
        let images = self.images.read().unwrap();
        (*images.clone()).to_vec()
    }

    pub fn remove(&mut self, key: String) -> Result<Image, ImageStoreErr> {
        let mut images = match self.images.write() {
            Ok(images) => images,
            Err(e) => {
                return Err(ImageStoreErr::new(&format!(
                    "Could not acquire store lock: {}",
                    e.to_string()
                )))
            }
        };
        for i in 0..images.len() {
            if images[i].id == key {
                return Ok(images.remove(i));
            }
        }
        return Err(ImageStoreErr::new(&format!("key {} not found", key)));
    }

    pub fn pull(&mut self, reference: ImageReference) -> Result<(), ImageStoreErr> {
        let pull_path = self.pull_path(reference);
        std::fs::create_dir_all(&pull_path).expect("could not create pull path");
        pull_wasm(
            reference,
            self.pull_file_path(reference),
        )?;
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

    pub(crate) fn pull_path(&self, image_ref: ImageReference) -> PathBuf {
        self.root_dir
            .join(image_ref.registry)
            .join(image_ref.repo)
            .join(image_ref.tag)
    }

    pub(crate) fn pull_file_path(&self, image_ref: ImageReference) -> PathBuf {
        self.pull_path(image_ref).join("module.wasm")
    }
}

fn pull_wasm(reference: ImageReference, fp: PathBuf) -> Result<(), ImageStoreErr> {
    println!("pulling {} into {}", reference.whole, fp.to_str().unwrap());
    let c_ref = CString::new(reference.whole).expect("CString::new failed");
    let c_file = CString::new(fp.to_str().unwrap()).expect("CString::new failed");

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
        _ => Err(ImageStoreErr::new("cannot pull module")),
    }
}

#[test]
fn test_pull_wasm() {
    use std::convert::TryFrom;

    // this is a public registry, so this test is both making sure the library is working,
    // as well as ensuring the registry is publicly accessible
    let module = "webassembly.azurecr.io/hello-wasm:v1".to_owned();
    let image_ref = ImageReference::try_from(&module).expect("Failed to parse image_ref");
    pull_wasm(image_ref, PathBuf::from("target/pulled.wasm")).unwrap();
}
