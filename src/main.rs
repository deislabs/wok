use tonic::{transport::Server, Request, Response, Status};

// RuntimeService is converted to a package runtime_service_server
use cri::{
    runtime_service_server::{RuntimeService, RuntimeServiceServer},
    VersionRequest, VersionResponse,
};

// Tonic will autogenerate the module's body.
pub mod cri {
    tonic::include_proto!("runtime.v1alpha2");
}

/// The version of the runtime API that this tool knows.
const RUNTIME_API_VERSION: &str = "v1alpha2";

type CriResult<T> = Result<Response<T>, Status>;

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
            runtime_api_version: "0.1.0-alpha.2".to_string(),
        }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:50051";
    let runtime = CRIRuntimeService {};
    env_logger::init();

    log::info!("listening on {}", addr);

    Server::builder()
        .add_service(RuntimeServiceServer::new(runtime))
        .serve(addr.parse()?)
        .await?;

    Ok(())
}
