use std::convert::TryFrom;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Utc;
use tonic::{Request, Response};

use super::grpc;

use crate::docker::Reference;
use crate::server::CriResult;
use crate::store::ModuleStore;
use crate::util;

/// Implement a CRI Image Service
#[derive(Debug, Default)]
pub struct CriImageService {
    module_store: Mutex<ModuleStore>,
}

impl CriImageService {
    pub fn new(root_dir: PathBuf) -> Self {
        util::ensure_root_dir(&root_dir).expect("cannot create root directory for image service");
        CriImageService {
            module_store: Mutex::new(ModuleStore::new(root_dir)),
        }
    }

    fn pull_module(&self, module_ref: Reference) -> Result<(), failure::Error> {
        self.module_store.lock().unwrap().pull(module_ref)?;
        Ok(())
    }
}

#[tonic::async_trait]
impl grpc::image_service_server::ImageService for CriImageService {
    async fn list_images(
        &self,
        _request: Request<grpc::ListImagesRequest>,
    ) -> CriResult<grpc::ListImagesResponse> {
        let resp = grpc::ListImagesResponse {
            images: self.module_store.lock().unwrap().list().unwrap(),
        };
        Ok(Response::new(resp))
    }

    async fn pull_image(
        &self,
        request: Request<grpc::PullImageRequest>,
    ) -> CriResult<grpc::PullImageResponse> {
        let image_ref = request.into_inner().image.unwrap().image;

        self.pull_module(Reference::try_from(&image_ref).expect("Image ref is malformed"))
            .expect("cannot pull module");
        let resp = grpc::PullImageResponse { image_ref };

        // TODO(bacongobbler): add to the image store

        Ok(Response::new(resp))
    }

    /// returns information of the filesystem that is used to store images.
    async fn image_fs_info(
        &self,
        _request: Request<grpc::ImageFsInfoRequest>,
    ) -> CriResult<grpc::ImageFsInfoResponse> {
        let module_store = self.module_store.lock().unwrap();
        let resp = grpc::ImageFsInfoResponse {
            image_filesystems: vec![grpc::FilesystemUsage {
                timestamp: Utc::now().timestamp_nanos(),
                fs_id: Some(grpc::FilesystemIdentifier {
                    mountpoint: module_store
                        .root_dir()
                        .clone()
                        .into_os_string()
                        .into_string()
                        .unwrap(),
                }),
                used_bytes: Some(grpc::UInt64Value {
                    value: module_store.used_bytes().unwrap(),
                }),
                inodes_used: Some(grpc::UInt64Value {
                    value: module_store.used_inodes().unwrap(),
                }),
            }],
        };
        Ok(Response::new(resp))
    }
}
