use tonic::{Request, Response, Status};
// RuntimeService is converted to a package runtime_service_server
use crate::grpc::{
    runtime_service_server::RuntimeService,
    PodSandbox,
    ListPodSandboxRequest, ListPodSandboxResponse,
    VersionRequest, VersionResponse,
    image_service_server::ImageService,
    Image,
    ListImagesRequest, ListImagesResponse,
    ImageStatusRequest, ImageStatusResponse,
    PullImageRequest, PullImageResponse,
    RemoveImageRequest, RemoveImageResponse,
    ImageFsInfoRequest, ImageFsInfoResponse,
};

/// The version of the runtime API that this tool knows.
/// See CRI-O for reference (since docs don't explain this)
/// https://github.com/cri-o/cri-o/blob/master/server/version.go
const RUNTIME_API_VERSION: &str = "v1alpha2";
/// The API version of this CRI plugin.
const API_VERSION: &str = "0.1.0";

/// CriResult describes a Result that has a Response<T> and a Status
pub type CriResult<T> = std::result::Result<Response<T>, Status>;

/// Result describes a Runtime result that may return a failure::Error if things go wrong.
pub type Result<T> = std::result::Result<T, failure::Error>;

/// Implement a CRI runtime service.
#[derive(Debug, Default)]
pub struct CriRuntimeService {
    pods: Vec<PodSandbox>,
    images: Vec<Image>,
}

impl CriRuntimeService {
    pub fn new() -> Self {
        CriRuntimeService { pods: vec![], images: vec![] }
    }
}

#[tonic::async_trait]
impl RuntimeService for CriRuntimeService {
    async fn version(&self, req: Request<VersionRequest>) -> CriResult<VersionResponse> {
        log::info!("Version request from API version {:?}", req);
        Ok(Response::new(VersionResponse {
            version: API_VERSION.to_string(),
            runtime_name: env!("CARGO_PKG_NAME").to_string(),
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
            // NOTE: The Kubernetes API distinctly says that this MUST be a SemVer...
            // but actually require this format, which is not SemVer at all.
            runtime_api_version: RUNTIME_API_VERSION.to_string(),
        }))
    }

    async fn list_pod_sandbox(
        &self,
        _req: Request<ListPodSandboxRequest>,
    ) -> CriResult<ListPodSandboxResponse> {
        Ok(Response::new(ListPodSandboxResponse {
            items: self.pods.clone(),
        }))
    }
}

#[tonic::async_trait]
impl ImageService for CriRuntimeService {
    async fn list_images(
        &self,
        _req: Request<ListImagesRequest>,
    ) -> CriResult<ListImagesResponse> {
        Ok(Response::new(ListImagesResponse {
            images: self.images.clone(),
        }))
    }

    async fn image_status(
        &self,
        _req: Request<ImageStatusRequest>,
    ) -> CriResult<ImageStatusResponse> {
        Err(tonic::Status::unimplemented("Not yet implemented"))
    }

    async fn pull_image(
        &self,
        _req: Request<PullImageRequest>,
    ) -> CriResult<PullImageResponse> {
        Err(tonic::Status::unimplemented("Not yet implemented"))
    }

    async fn remove_image(
        &self,
        _req: Request<RemoveImageRequest>,
    ) -> CriResult<RemoveImageResponse> {
        Err(tonic::Status::unimplemented("Not yet implemented"))
    }

    async fn image_fs_info(
        &self,
        _req: Request<ImageFsInfoRequest>,
    ) -> CriResult<ImageFsInfoResponse> {
        Err(tonic::Status::unimplemented("Not yet implemented"))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::grpc::{ListPodSandboxRequest, VersionRequest};
    use futures::executor::block_on;
    use tonic::Request;
    #[test]
    fn test_version() {
        block_on(_test_version())
    }

    #[test]
    fn test_list_pod_sandbox() {
        block_on(_test_list_pod_sandbox())
    }

    #[test]
    fn test_list_images() {
        block_on(_test_list_images())
    }

    #[test]
    fn test_image_status() {
        block_on(_test_image_status())
    }

    #[test]
    fn test_pull_image() {
        block_on(_test_pull_image())
    }

    #[test]
    fn test_remove_image() {
        block_on(_test_remove_image())
    }

    #[test]
    fn test_image_fs_info() {
        block_on(_test_image_fs_info())
    }

    async fn _test_version() {
        let svc = CriRuntimeService::new();
        let res = svc.version(Request::new(VersionRequest::default())).await;
        assert_eq!(
            res.as_ref()
                .expect("successful version request")
                .get_ref()
                .version,
            API_VERSION
        );
        assert_eq!(
            res.expect("successful version request")
                .get_ref()
                .runtime_api_version,
            RUNTIME_API_VERSION
        );
    }

    async fn _test_list_pod_sandbox() {
        let svc = CriRuntimeService::new();
        let req = Request::new(ListPodSandboxRequest::default());
        let res = svc.list_pod_sandbox(req).await;
        assert_eq!(0, res.expect("successful pod list").get_ref().items.len());
    }

    async fn _test_list_images() {
        let svc = CriRuntimeService::new();
        let req = Request::new(ListImagesRequest::default());
        let res = svc.list_images(req).await;
        assert_eq!(0, res.expect("successful image list").get_ref().images.len());
    }

    async fn _test_image_status() {
        let svc = CriRuntimeService::new();
        let req = Request::new(ImageStatusRequest::default());
        let res = svc.image_status(req).await;
        assert!(res.is_err(), "successful image status");
    }

    async fn _test_pull_image() {
        let svc = CriRuntimeService::new();
        let req = Request::new(PullImageRequest::default());
        let res = svc.pull_image(req).await;
        assert!(res.is_err(), "successful image pull");
    }

    async fn _test_remove_image() {
        let svc = CriRuntimeService::new();
        let req = Request::new(RemoveImageRequest::default());
        let res = svc.remove_image(req).await;
        assert!(res.is_err(), "successful image remove");
    }

    async fn _test_image_fs_info() {
        let svc = CriRuntimeService::new();
        let req = Request::new(ImageFsInfoRequest::default());
        let res = svc.image_fs_info(req).await;
        assert!(res.is_err(), "successful image fs info");
    }
}
