use std::convert::TryFrom;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Utc;
use tonic::{Request, Response};

use super::grpc::{
    image_service_server::ImageService, FilesystemIdentifier, FilesystemUsage, ImageFsInfoRequest,
    ImageFsInfoResponse, ListImagesRequest, ListImagesResponse, PullImageRequest,
    PullImageResponse, UInt64Value,
};

use crate::reference::Reference;
use crate::server::CriResult;
use crate::store::ImageStore;
use crate::util;

/// Implement a CRI Image Service
#[derive(Debug, Default)]
pub struct CriImageService {
    image_store: Mutex<ImageStore>,
}

impl CriImageService {
    pub fn new(root_dir: PathBuf) -> Self {
        util::ensure_root_dir(&root_dir).expect("cannot create root directory for image service");
        CriImageService {
            image_store: Mutex::new(ImageStore::new(root_dir)),
        }
    }

    fn pull_module(&self, module_ref: Reference) -> Result<(), failure::Error> {
        self.image_store.lock().unwrap().pull(module_ref)?;
        Ok(())
    }
}

#[tonic::async_trait]
impl ImageService for CriImageService {
    async fn list_images(
        &self,
        _request: Request<ListImagesRequest>,
    ) -> CriResult<ListImagesResponse> {
        let resp = ListImagesResponse {
            images: self.image_store.lock().unwrap().list(),
        };
        Ok(Response::new(resp))
    }

    async fn pull_image(&self, request: Request<PullImageRequest>) -> CriResult<PullImageResponse> {
        let image_ref = request.into_inner().image.unwrap().image;

        self.pull_module(Reference::try_from(&image_ref).expect("Image ref is malformed"))
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
        let image_store = self.image_store.lock().unwrap();
        let resp = ImageFsInfoResponse {
            image_filesystems: vec![FilesystemUsage {
                timestamp: Utc::now().timestamp_nanos(),
                fs_id: Some(FilesystemIdentifier {
                    mountpoint: image_store
                        .root_dir()
                        .clone()
                        .into_os_string()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                }),
                used_bytes: Some(UInt64Value {
                    value: image_store.used_bytes(),
                }),
                inodes_used: Some(UInt64Value {
                    value: image_store.used_inodes(),
                }),
            }],
        };
        Ok(Response::new(resp))
    }
}
