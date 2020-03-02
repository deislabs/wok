use std::collections::HashMap;
use std::convert::TryFrom;
use std::path::PathBuf;

use chrono::Utc;
use tokio::sync::Mutex;
use tonic::{Request, Response};

use super::grpc;

use crate::docker::Reference;
use crate::server::CriResult;
use crate::store::ModuleStore;

/// Implement a CRI Image Service
#[derive(Debug, Default)]
pub struct CriImageService {
    module_store: Mutex<ModuleStore>,
}

impl CriImageService {
    pub async fn new(root_dir: PathBuf) -> Self {
        tokio::fs::create_dir_all(&root_dir)
            .await
            .expect("cannot create root directory for image service");
        CriImageService {
            module_store: Mutex::new(ModuleStore::new(root_dir).await),
        }
    }

    async fn pull_module(&self, module_ref: Reference) -> Result<(), failure::Error> {
        self.module_store.lock().await.pull(&module_ref).await?;

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
            images: self.module_store.lock().await.list().await,
        };
        Ok(Response::new(resp))
    }

    async fn image_status(
        &self,
        req: Request<grpc::ImageStatusRequest>,
    ) -> CriResult<grpc::ImageStatusResponse> {
        // TODO(rylev): handle error of image in request not being there.
        let image_id = req.into_inner().image.unwrap().image;
        let image = self
            .module_store
            .lock()
            .await
            .list()
            .await
            .iter()
            .find(|i| i.id == image_id)
            .cloned();

        let resp = grpc::ImageStatusResponse {
            image,
            info: HashMap::new(), // TODO(rylev): figure out what goes here
        };
        Ok(Response::new(resp))
    }

    async fn pull_image(
        &self,
        request: Request<grpc::PullImageRequest>,
    ) -> CriResult<grpc::PullImageResponse> {
        let image_ref = request.into_inner().image.unwrap().image;
        let reference = Reference::try_from(image_ref.clone()).expect("Image ref is malformed");
        self.pull_module(reference)
            .await
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
        let module_store = self.module_store.lock().await;
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
                    value: module_store.used_bytes().await,
                }),
                inodes_used: Some(grpc::UInt64Value {
                    value: module_store.used_inodes().await,
                }),
            }],
        };
        Ok(Response::new(resp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grpc::image_service_server::ImageService;

    #[tokio::test]
    async fn test_image_status() {
        let service = CriImageService::new(PathBuf::from("")).await;
        let req = grpc::ImageStatusRequest {
            image: Some(grpc::ImageSpec {
                image: "foo/bar:baz".to_owned(),
            }),
            verbose: false,
        };
        let response = service.image_status(Request::new(req)).await;
        assert!(response.unwrap().into_inner().image.is_none());
    }
}
