use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::reference::Reference;
use crate::server::Image;

#[derive(Clone, Debug, Default)]
pub struct ImageStore {
    root_dir: PathBuf,
    images: Arc<RwLock<Vec<Image>>>,
}

/// An error which can be returned when there was an error
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
                    e.to_string()
                )))
            }
        };
        (*images).push(image);
        Ok(())
    }

    pub fn root_dir(&self) -> PathBuf {
        self.root_dir.clone()
    }

    pub fn get(&self, key: String) -> Option<Image> {
        let images = self.images.read().unwrap();
        images.iter().cloned().find(|x| x.id == key)
    }

    pub fn list(&self) -> Vec<Image> {
        let images = self.images.read().unwrap();
        (*images.to_owned()).to_vec()
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

    pub(crate) fn used_bytes(&self) -> u64 {
        let mut used: u64 = 0;
        let images = self.images.read().unwrap();
        for image in images.iter() {
            used += image.size
        }
        used
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
