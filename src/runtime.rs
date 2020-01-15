use std::collections::HashMap;
use tonic::{Request, Response, Status};
// RuntimeService is converted to a package runtime_service_server
use crate::grpc::{
    runtime_service_server::RuntimeService, ListPodSandboxRequest, ListPodSandboxResponse,
    PodSandbox, PodSandboxStatusRequest, PodSandboxStatusResponse, RemovePodSandboxRequest,
    RemovePodSandboxResponse, RunPodSandboxRequest, RunPodSandboxResponse, StopPodSandboxRequest,
    StopPodSandboxResponse, VersionRequest, VersionResponse,
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
}

impl CriRuntimeService {
    pub fn new() -> Self {
        CriRuntimeService { pods: vec![] }
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

    async fn run_pod_sandbox(
        &self,
        _req: Request<RunPodSandboxRequest>,
    ) -> CriResult<RunPodSandboxResponse> {
        Ok(Response::new(RunPodSandboxResponse {
            pod_sandbox_id: "1".to_owned(),
        }))
    }

    async fn stop_pod_sandbox(
        &self,
        _req: Request<StopPodSandboxRequest>,
    ) -> CriResult<StopPodSandboxResponse> {
        Ok(Response::new(StopPodSandboxResponse {}))
    }

    async fn remove_pod_sandbox(
        &self,
        _req: Request<RemovePodSandboxRequest>,
    ) -> CriResult<RemovePodSandboxResponse> {
        Ok(Response::new(RemovePodSandboxResponse {}))
    }

    async fn pod_sandbox_status(
        &self,
        _req: Request<PodSandboxStatusRequest>,
    ) -> CriResult<PodSandboxStatusResponse> {
        Ok(Response::new(PodSandboxStatusResponse {
            info: HashMap::new(),
            status: None,
        }))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::grpc::*;
    use futures::executor::block_on;
    use tonic::Request;
    #[test]
    fn test_version() {
        block_on(_test_version())
    }

    #[test]
    fn test_run_pod_sandbox() {
        block_on(_test_run_pod_sandbox())
    }

    #[test]
    fn test_list_pod_sandbox() {
        block_on(_test_list_pod_sandbox())
    }

    #[test]
    fn test_pod_sandbox_status() {
        block_on(_test_pod_sandbox_status())
    }

    #[test]
    fn test_remove_pod_sandbox() {
        block_on(_test_remove_pod_sandbox())
    }

    #[test]
    fn test_stop_pod_sandbox() {
        block_on(_test_stop_pod_sandbox())
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

    async fn _test_run_pod_sandbox() {
        let svc = CriRuntimeService::new();
        let req = Request::new(RunPodSandboxRequest::default());
        let res = svc.run_pod_sandbox(req).await;
        assert_eq!(
            "1".to_owned(),
            res.expect("successful pod run submission")
                .get_ref()
                .pod_sandbox_id
        );
    }

    async fn _test_list_pod_sandbox() {
        let svc = CriRuntimeService::new();
        let req = Request::new(ListPodSandboxRequest::default());
        let res = svc.list_pod_sandbox(req).await;
        assert_eq!(0, res.expect("successful pod list").get_ref().items.len());
    }

    async fn _test_pod_sandbox_status() {
        let svc = CriRuntimeService::new();
        let req = Request::new(PodSandboxStatusRequest::default());
        let res = svc.pod_sandbox_status(req).await;
        assert_eq!(None, res.expect("status result").get_ref().status);
    }

    async fn _test_remove_pod_sandbox() {
        let svc = CriRuntimeService::new();
        let req = Request::new(RemovePodSandboxRequest::default());
        let res = svc.remove_pod_sandbox(req).await;
        // We expect an empty response object
        res.expect("remove sandbox result");
    }

    async fn _test_stop_pod_sandbox() {
        let svc = CriRuntimeService::new();
        let req = Request::new(StopPodSandboxRequest {
            pod_sandbox_id: "test".to_owned(),
        });
        let res = svc.stop_pod_sandbox(req).await;

        // Expect the stopped ID to be the same as the requested ID.
        res.expect("empty stop result");
    }
}
