use tonic::{Request, Response, Status};

// The RuntimeService grpc is converted to a package runtime_service_server
use crate::grpc::{
    runtime_service_server::RuntimeService, RunPodSandboxRequest, RunPodSandboxResponse,
    VersionRequest, VersionResponse,
};
use crate::{CriResult, Result};

/// The version of the runtime API that this tool knows.
/// See CRI-O for reference (since docs don't explain this)
/// https://github.com/cri-o/cri-o/blob/master/server/version.go
pub const RUNTIME_API_VERSION: &str = "0.1.0";

/// Implement a CRI runtime service.
#[derive(Debug, Default)]
pub struct CRIRuntimeService {}

pub enum RuntimeHandler {
    WASI,
    WASCC,
}

impl RuntimeHandler {
    pub fn from_string(s: &str) -> Result<Self> {
        match s {
            // Per the spec, the empty string should use the default
            "" => Ok(Self::default()),
            "WASI" => Ok(Self::WASI),
            "WASCC" => Ok(Self::WASCC),
            _ => Err(format_err!("Invalid runtime handler {}", s)),
        }
    }
}

impl Default for RuntimeHandler {
    fn default() -> Self {
        Self::WASI
    }
}

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

    async fn run_pod_sandbox(
        &self,
        req: Request<RunPodSandboxRequest>,
    ) -> CriResult<RunPodSandboxResponse> {
        let sandbox_req = req.into_inner();
        let handler = RuntimeHandler::from_string(&sandbox_req.runtime_handler)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        // NOTE: As of now, there isn't networking support in wasmtime, so we
        // can't necessarily set it up right now
        Ok(Response::new(RunPodSandboxResponse {
            pod_sandbox_id: String::default(),
        }))
    }
}
