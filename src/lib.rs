use tonic::{Request, Response, Status};
#[macro_use]
extern crate failure;

// RuntimeService is converted to a package runtime_service_server
use grpc::{
    runtime_service_server::RuntimeService, ListPodSandboxRequest, ListPodSandboxResponse,
    PodSandbox, VersionRequest, VersionResponse,
};

pub mod wasm;

// Tonic will autogenerate the module's body.
pub mod grpc {
    tonic::include_proto!("runtime.v1alpha2");
}

pub mod oci;

/// The version of the runtime API that this tool knows.
/// See CRI-O for reference (since docs don't explain this)
/// https://github.com/cri-o/cri-o/blob/master/server/version.go
const RUNTIME_API_VERSION: &str = "v1alpha2";
/// The API version of this CRI plugin.
const API_VERSION: &str = "0.1.0";

type CriResult<T> = std::result::Result<Response<T>, Status>;

type Result<T> = std::result::Result<T, failure::Error>;

/// Implement a CRI runtime service.
#[derive(Debug, Default)]
pub struct CRIRuntimeService {
    pods: Vec<PodSandbox>,
}

impl CRIRuntimeService {
    pub fn new() -> Self {
        CRIRuntimeService { pods: vec![] }
    }
}

#[tonic::async_trait]
impl RuntimeService for CRIRuntimeService {
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

    async fn _test_version() {
        let svc = CRIRuntimeService::new();
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
        let svc = CRIRuntimeService::new();
        let req = Request::new(ListPodSandboxRequest::default());
        let res = svc.list_pod_sandbox(req).await;
        assert_eq!(0, res.expect("successful pod list").get_ref().items.len());
    }
}
