use tonic::{Request, Response};

// The RuntimeService grpc is converted to a package runtime_service_server
use crate::grpc::{runtime_service_server::RuntimeService, VersionRequest, VersionResponse};
use crate::CriResult;

/// The version of the runtime API that this tool knows.
/// See CRI-O for reference (since docs don't explain this)
/// https://github.com/cri-o/cri-o/blob/master/server/version.go
pub const RUNTIME_API_VERSION: &str = "0.1.0";

/// Implement a CRI runtime service.
#[derive(Debug, Default)]
pub struct CRIRuntimeService {}

#[tonic::async_trait]
impl RuntimeService for CRIRuntimeService {
    async fn version(&self, req: Request<VersionRequest>) -> CriResult<VersionResponse> {
        log::info!("Version request from API version {:?}", req);
        Ok(Response::new(VersionResponse {
            version: RUNTIME_API_VERSION.to_string(),
            runtime_name: env!("CARGO_PKG_NAME").to_string(),
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
            // NOTE: The Kubernetes API distinctly says that this MUST be a SemVer...
            // but actually require this format, which is not SemVer at all.
            runtime_api_version: "v1alpha2".to_string(),
        }))
    }
}
