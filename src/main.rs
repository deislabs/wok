use tonic::{transport::Server, Request, Response, Status};

// RuntimeService is converted to a package runtime_service_server
use cri::{runtime_service_server::RuntimeService, VersionRequest, VersionResponse};

// Tonic will autogenerate the module's body.
pub mod cri {
    tonic::include_proto!("runtime.v1alpha2");
}

/// The version of the runtime API that this tool knows.
const RUNTIME_API_VERSION: &str = "v1alpha2";

type CriResult<T> = Result<Response<T>, Status>;

/// Implement a CRI runtime service.
pub struct CRIRuntimeService {}

#[tonic::async_trait]
impl RuntimeService for CRIRuntimeService {
    async fn version(&self, req: Request<VersionRequest>) -> CriResult<VersionResponse> {
        log::info!("Version request from API version {:?}", req);
        Ok(Response::new(VersionResponse {
            version: RUNTIME_API_VERSION.to_string(),
            runtime_name: "wok".to_string(),
            runtime_version: "0.1.0".to_string(),
            runtime_api_version: "0.1.0-alpha.2".to_string(),
        }))
    }
}

fn main() {
    println!("Hello, world!");
}
