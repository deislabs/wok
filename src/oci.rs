use crate::grpc::{
    image_service_server::ImageService, FilesystemIdentifier, FilesystemUsage, Image,
    ImageFsInfoRequest, ImageFsInfoResponse, ListImagesRequest, ListImagesResponse,
    PullImageRequest, PullImageResponse, UInt64Value,
};
use chrono::Utc;
use std::convert::TryFrom;
use std::ffi::CString;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tonic::{Request, Response};

use crate::runtime::CriResult;
use crate::util;

pub fn default_image_dir() -> PathBuf {
    dirs::home_dir()
        .expect("cannot get home directory")
        .join(".wok")
        .join("modules")
}

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

    pub(crate) fn pull_path(&self, image_ref: ImageRef) -> PathBuf {
        self.root_dir
            .join(image_ref.registry)
            .join(image_ref.repo)
            .join(image_ref.tag)
    }

    pub(crate) fn pull_file_path(&self, image_ref: ImageRef) -> PathBuf {
        self.pull_path(image_ref).join("module.wasm")
    }
}

/// Implement a CRI Image Service
#[derive(Debug, Default)]
pub struct CriImageService {
    image_store: ImageStore,
}

impl CriImageService {
    pub fn new(root_dir: PathBuf) -> Self {
        util::ensure_root_dir(&root_dir).expect("cannot create root directory for image service");
        CriImageService {
            image_store: ImageStore::new(root_dir),
        }
    }

    fn pull_module(&self, module_ref: ImageRef) -> Result<(), failure::Error> {
        let pull_path = self.image_store.pull_path(module_ref);
        std::fs::create_dir_all(&pull_path)?;
        pull_wasm(
            module_ref.whole,
            self.image_store
                .pull_file_path(module_ref)
                .to_str()
                .unwrap(),
        )
    }
}

// currently, the library only accepts modules tagged in the following structure:
// <registry>/<repository>:<tag>
// for example: webassembly.azurecr.io/hello:v1
#[derive(Copy, Clone)]
pub(crate) struct ImageRef<'a> {
    pub(crate) whole: &'a str,
    pub(crate) registry: &'a str,
    pub(crate) repo: &'a str,
    pub(crate) tag: &'a str,
}

impl<'a> TryFrom<&'a String> for ImageRef<'a> {
    type Error = ();
    fn try_from(string: &'a String) -> Result<Self, Self::Error> {
        let mut registry_parts = string.split('/');
        let registry = registry_parts.next().ok_or(())?;
        let mut repo_parts = registry_parts.next().ok_or(())?.split(':');
        let repo = repo_parts.next().ok_or(())?;
        let tag = repo_parts.next().ok_or(())?;
        Ok(ImageRef {
            whole: string,
            registry,
            repo,
            tag,
        })
    }
}

#[tonic::async_trait]
impl ImageService for CriImageService {
    async fn list_images(
        &self,
        _request: Request<ListImagesRequest>,
    ) -> CriResult<ListImagesResponse> {
        let resp = ListImagesResponse {
            images: self.image_store.list(),
        };
        Ok(Response::new(resp))
    }

    async fn pull_image(&self, request: Request<PullImageRequest>) -> CriResult<PullImageResponse> {
        let image_ref = request.into_inner().image.unwrap().image;

        self.pull_module(ImageRef::try_from(&image_ref).expect("Image ref is malformed"))
            .expect("cannot pull module");
        let resp = PullImageResponse { image_ref };

        // TODO(bacongobbler): add to the image store

        Ok(Response::new(resp))
    }

    /// returns information of the filesystem that is used to store images.
    async fn image_fs_info(
        &self,
        _request: Request<ImageFsInfoRequest>,
    ) -> CriResult<ImageFsInfoResponse> {
        let resp = ImageFsInfoResponse {
            image_filesystems: vec![FilesystemUsage {
                timestamp: Utc::now().timestamp_nanos(),
                fs_id: Some(FilesystemIdentifier {
                    mountpoint: self
                        .image_store
                        .root_dir
                        .clone()
                        .into_os_string()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                }),
                used_bytes: Some(UInt64Value {
                    value: self.image_store.used_bytes(),
                }),
                inodes_used: Some(UInt64Value {
                    value: self.image_store.used_inodes(),
                }),
            }],
        };
        Ok(Response::new(resp))
    }
}

pub fn pull_wasm(reference: &str, file: &str) -> Result<(), failure::Error> {
    println!("pulling {} into {}", reference, file);
    let c_ref = CString::new(reference)?;
    let c_file = CString::new(file)?;

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
        _ => Err(failure::Error::from(OCIError::Custom(
            "cannot pull module".into(),
        ))),
    }
}

#[test]
fn test_pull_wasm() {
    // this is a public registry, so this test is both making sure the library is working,
    // as well as ensuring the registry is publicly accessible
    let module = "webassembly.azurecr.io/hello-wasm:v1";
    pull_wasm(module, "target/pulled.wasm").unwrap();
}

#[derive(Debug)]
pub enum OCIError {
    Custom(String),
    Io(std::io::Error),
    Nul(std::ffi::NulError),
}

impl From<std::io::Error> for OCIError {
    fn from(err: std::io::Error) -> Self {
        OCIError::Io(err)
    }
}

impl From<std::ffi::NulError> for OCIError {
    fn from(err: std::ffi::NulError) -> Self {
        OCIError::Nul(err)
    }
}

impl std::fmt::Display for OCIError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}

impl std::error::Error for OCIError {
    fn cause(&self) -> Option<&dyn std::error::Error> {
        Some(self)
    }
}

/* automatically generated by rust-bindgen */

#[derive(PartialEq, Copy, Clone, Hash, Debug, Default)]
#[repr(C)]
pub struct __BindgenComplex<T> {
    pub re: T,
    pub im: T,
}
#[allow(non_camel_case_types)]
pub type wchar_t = ::std::os::raw::c_int;
#[allow(non_camel_case_types)]
pub type max_align_t = u128;
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct _GoString_ {
    pub p: *const ::std::os::raw::c_char,
    pub n: isize,
}
#[test]
#[allow(non_snake_case)]
fn bindgen_test_layout__GoString_() {
    assert_eq!(
        ::std::mem::size_of::<_GoString_>(),
        16usize,
        concat!("Size of: ", stringify!(_GoString_))
    );
    assert_eq!(
        ::std::mem::align_of::<_GoString_>(),
        8usize,
        concat!("Alignment of ", stringify!(_GoString_))
    );
    assert_eq!(
        unsafe { &(*(::std::ptr::null::<_GoString_>())).p as *const _ as usize },
        0usize,
        concat!(
            "Offset of field: ",
            stringify!(_GoString_),
            "::",
            stringify!(p)
        )
    );
    assert_eq!(
        unsafe { &(*(::std::ptr::null::<_GoString_>())).n as *const _ as usize },
        8usize,
        concat!(
            "Offset of field: ",
            stringify!(_GoString_),
            "::",
            stringify!(n)
        )
    );
}
pub type GoInt8 = ::std::os::raw::c_schar;
pub type GoUint8 = ::std::os::raw::c_uchar;
pub type GoInt16 = ::std::os::raw::c_short;
pub type GoUint16 = ::std::os::raw::c_ushort;
pub type GoInt32 = ::std::os::raw::c_int;
pub type GoUint32 = ::std::os::raw::c_uint;
pub type GoInt64 = ::std::os::raw::c_longlong;
pub type GoUint64 = ::std::os::raw::c_ulonglong;
pub type GoInt = GoInt64;
pub type GoUint = GoUint64;
pub type GoUintptr = ::std::os::raw::c_ulong;
pub type GoFloat32 = f32;
pub type GoFloat64 = f64;
pub type GoComplex64 = __BindgenComplex<f32>;
pub type GoComplex128 = __BindgenComplex<f64>;
#[allow(non_camel_case_types)]
pub type _check_for_64_bit_pointer_matching_GoInt = [::std::os::raw::c_char; 1usize];
pub type GoString = _GoString_;
pub type GoMap = *mut ::std::os::raw::c_void;
pub type GoChan = *mut ::std::os::raw::c_void;
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct GoInterface {
    pub t: *mut ::std::os::raw::c_void,
    pub v: *mut ::std::os::raw::c_void,
}
#[test]
#[allow(non_snake_case)]
fn bindgen_test_layout_GoInterface() {
    assert_eq!(
        ::std::mem::size_of::<GoInterface>(),
        16usize,
        concat!("Size of: ", stringify!(GoInterface))
    );
    assert_eq!(
        ::std::mem::align_of::<GoInterface>(),
        8usize,
        concat!("Alignment of ", stringify!(GoInterface))
    );
    assert_eq!(
        unsafe { &(*(::std::ptr::null::<GoInterface>())).t as *const _ as usize },
        0usize,
        concat!(
            "Offset of field: ",
            stringify!(GoInterface),
            "::",
            stringify!(t)
        )
    );
    assert_eq!(
        unsafe { &(*(::std::ptr::null::<GoInterface>())).v as *const _ as usize },
        8usize,
        concat!(
            "Offset of field: ",
            stringify!(GoInterface),
            "::",
            stringify!(v)
        )
    );
}
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct GoSlice {
    pub data: *mut ::std::os::raw::c_void,
    pub len: GoInt,
    pub cap: GoInt,
}
#[allow(non_snake_case)]
#[test]
fn bindgen_test_layout_GoSlice() {
    assert_eq!(
        ::std::mem::size_of::<GoSlice>(),
        24usize,
        concat!("Size of: ", stringify!(GoSlice))
    );
    assert_eq!(
        ::std::mem::align_of::<GoSlice>(),
        8usize,
        concat!("Alignment of ", stringify!(GoSlice))
    );
    assert_eq!(
        unsafe { &(*(::std::ptr::null::<GoSlice>())).data as *const _ as usize },
        0usize,
        concat!(
            "Offset of field: ",
            stringify!(GoSlice),
            "::",
            stringify!(data)
        )
    );
    assert_eq!(
        unsafe { &(*(::std::ptr::null::<GoSlice>())).len as *const _ as usize },
        8usize,
        concat!(
            "Offset of field: ",
            stringify!(GoSlice),
            "::",
            stringify!(len)
        )
    );
    assert_eq!(
        unsafe { &(*(::std::ptr::null::<GoSlice>())).cap as *const _ as usize },
        16usize,
        concat!(
            "Offset of field: ",
            stringify!(GoSlice),
            "::",
            stringify!(cap)
        )
    );
}
extern "C" {
    pub fn Pull(p0: GoString, p1: GoString) -> GoInt64;
}
