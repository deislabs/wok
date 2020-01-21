use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};
use tonic::{Request, Response, Status};
// RuntimeService is converted to a package runtime_service_server
use crate::grpc::{
    runtime_service_server::RuntimeService, Container, ContainerMetadata, ContainerStatus,
    ContainerStatusRequest, ContainerStatusResponse, CreateContainerRequest,
    CreateContainerResponse, ImageSpec, ListContainersRequest, ListContainersResponse,
    ListPodSandboxRequest, ListPodSandboxResponse, PodSandbox, PodSandboxState,
    PodSandboxStatusRequest, PodSandboxStatusResponse, RemoveContainerRequest,
    RemoveContainerResponse, RemovePodSandboxRequest, RemovePodSandboxResponse,
    RunPodSandboxRequest, RunPodSandboxResponse, StartContainerRequest, StartContainerResponse,
    StopContainerRequest, StopContainerResponse, StopPodSandboxRequest, StopPodSandboxResponse,
    VersionRequest, VersionResponse,
};
use chrono::Utc;
use uuid::Uuid;

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
    // NOTE: we could replace this with evmap or crossbeam
    sandboxes: Arc<RwLock<BTreeMap<String, PodSandbox>>>,
    containers: Vec<Container>,
}

impl CriRuntimeService {
    pub fn new() -> Self {
        CriRuntimeService {
            sandboxes: Arc::new(RwLock::new(BTreeMap::default())),
            containers: vec![],
        }
    }
}

#[derive(Debug)]
pub enum RuntimeHandler {
    WASI,
    WASCC,
}

impl ToString for RuntimeHandler {
    fn to_string(&self) -> String {
        match self {
            Self::WASI => "WASI".to_owned(),
            Self::WASCC => "WASCC".to_owned(),
        }
    }
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

    async fn run_pod_sandbox(
        &self,
        req: Request<RunPodSandboxRequest>,
    ) -> CriResult<RunPodSandboxResponse> {
        let sandbox_req = req.into_inner();
        let sandbox_conf = match sandbox_req.config {
            Some(c) => c,
            None => {
                return Err(Status::invalid_argument(
                    "Sandbox request is missing config object",
                ))
            }
        };
        let handler = RuntimeHandler::from_string(&sandbox_req.runtime_handler)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        // TODO(taylor): As of now, there isn't networking support in wasmtime,
        // so we can't necessarily set it up right now. Once it does, we'll need
        // to set up networking here

        // Create the logs directory for this pod
        std::fs::create_dir_all(sandbox_conf.log_directory.clone())
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        // Basically, everything above here is all we need to set up a sandbox.
        // All of the security context stuff pretty much doesn't matter for
        // WASM, but we can revisit this as things keep evolving

        // Ok, now let's store things in memory
        let sandbox_map = self.sandboxes.clone();
        // TODO(taylor): According to the RwLock docs, we should panic on failure
        // to poison the RwLock. This also means we'd need to have handling for
        // recovering from a poisoned RwLock, which I am leaving for later
        let mut sandboxes = sandbox_map
            .write()
            .map_err(|e| {
                Status::internal(format!(
                    "Data inconsistency when trying to store sandbox data: {}",
                    e.to_string()
                ))
            })
            .unwrap();
        let id = Uuid::new_v4().to_string();
        sandboxes.insert(
            id.clone(),
            PodSandbox {
                id: id.clone(),
                metadata: sandbox_conf.metadata,
                state: PodSandboxState::SandboxReady as i32,
                created_at: Utc::now().timestamp_nanos(),
                labels: sandbox_conf.labels,
                annotations: sandbox_conf.annotations,
                runtime_handler: handler.to_string(),
            },
        );
        Ok(Response::new(RunPodSandboxResponse { pod_sandbox_id: id }))
    }

    async fn list_pod_sandbox(
        &self,
        _req: Request<ListPodSandboxRequest>,
    ) -> CriResult<ListPodSandboxResponse> {
        Ok(Response::new(ListPodSandboxResponse {
            items: self.sandboxes.read().unwrap().values().cloned().collect(),
        }))
    }

    async fn stop_pod_sandbox(
        &self,
        _req: Request<StopPodSandboxRequest>,
    ) -> CriResult<StopPodSandboxResponse> {
        Ok(Response::new(StopPodSandboxResponse {}))
    }

    // remove_pod_sandbox removes the sandbox. If there are running containers in the sandbox, they should be forcibly
    // removed.
    async fn remove_pod_sandbox(
        &self,
        req: Request<RemovePodSandboxRequest>,
    ) -> CriResult<RemovePodSandboxResponse> {
        let id = &req.into_inner().pod_sandbox_id;

        // wrap in a block so the RWLock will drop out of scope.
        {
            let sandboxes = self.sandboxes.read().unwrap();
            let sandbox = match sandboxes.get(id) {
                Some(s) => s,
                None => return Err(Status::not_found(format!("Sandbox {} does not exist", id))),
            };

            // return an error if the sandbox container is still running.
            if sandbox.state == PodSandboxState::SandboxReady as i32 {
                return Err(Status::failed_precondition(format!(
                    "Sandbox container {} is not fully stopped",
                    id
                )));
            }
        }

        // TODO(bacongobbler): when networking is implemented, here is where we should return an error if the sandbox's
        // network namespace is not closed yet.

        // remove all containers inside the sandbox.
        for container in &self.containers {
            if &container.pod_sandbox_id == id {
                self.remove_container(Request::new(RemoveContainerRequest {
                    container_id: container.id.to_owned(),
                }));
            }
        }

        // remove the sandbox.
        self.sandboxes.write().unwrap().remove(id);

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

    async fn create_container(
        &self,
        _req: Request<CreateContainerRequest>,
    ) -> CriResult<CreateContainerResponse> {
        Ok(Response::new(CreateContainerResponse {
            container_id: "1".to_owned(),
        }))
    }

    async fn start_container(
        &self,
        _req: Request<StartContainerRequest>,
    ) -> CriResult<StartContainerResponse> {
        Ok(Response::new(StartContainerResponse {}))
    }

    async fn stop_container(
        &self,
        _req: Request<StopContainerRequest>,
    ) -> CriResult<StopContainerResponse> {
        Ok(Response::new(StopContainerResponse {}))
    }

    async fn remove_container(
        &self,
        _req: Request<RemoveContainerRequest>,
    ) -> CriResult<RemoveContainerResponse> {
        Ok(Response::new(RemoveContainerResponse {}))
    }

    async fn list_containers(
        &self,
        _req: Request<ListContainersRequest>,
    ) -> CriResult<ListContainersResponse> {
        Ok(Response::new(ListContainersResponse {
            containers: self.containers.clone(),
        }))
    }

    async fn container_status(
        &self,
        _req: Request<ContainerStatusRequest>,
    ) -> CriResult<ContainerStatusResponse> {
        Ok(Response::new(ContainerStatusResponse {
            status: Some(ContainerStatus {
                id: "1".to_owned(),
                metadata: Some(ContainerMetadata {
                    name: "foo".to_owned(),
                    attempt: 0,
                }),
                state: 0,
                created_at: 0,
                started_at: 0,
                finished_at: 0,
                exit_code: 0,
                image: Some(ImageSpec {
                    image: "foo".to_owned(),
                }),
                image_ref: "foo".to_owned(),
                reason: "because I said so".to_owned(),
                message: "hello earthlings".to_owned(),
                labels: HashMap::new(),
                annotations: HashMap::new(),
                mounts: vec![],
                log_path: "/dev/null".to_owned(),
            }),
            info: HashMap::new(),
        }))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::grpc::*;
    use futures::executor::block_on;
    use tempfile::tempdir;
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
    fn test_create_and_list() {
        block_on(_test_create_and_list())
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

    #[test]
    fn test_create_container() {
        block_on(_test_create_container())
    }

    #[test]
    fn test_start_container() {
        block_on(_test_start_container())
    }

    #[test]
    fn test_stop_container() {
        block_on(_test_stop_container())
    }

    #[test]
    fn test_remove_container() {
        block_on(_test_remove_container())
    }

    #[test]
    fn test_list_containers() {
        block_on(_test_list_containers())
    }

    #[test]
    fn test_container_status() {
        block_on(_test_container_status())
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

    async fn _test_pod_sandbox_status() {
        let svc = CriRuntimeService::new();
        let req = Request::new(PodSandboxStatusRequest::default());
        let res = svc.pod_sandbox_status(req).await;
        assert_eq!(None, res.expect("status result").get_ref().status);
    }

    async fn _test_remove_pod_sandbox() {
        let mut svc = CriRuntimeService::new();
        let mut container = Container::default();
        container.pod_sandbox_id = "1".to_owned();
        svc.containers.push(container);
        {
            let mut sandboxes = svc.sandboxes.write().unwrap();
            sandboxes.insert(
                "1".to_owned(),
                PodSandbox {
                    id: "1".to_owned(),
                    metadata: None,
                    state: PodSandboxState::SandboxNotready as i32,
                    created_at: Utc::now().timestamp_nanos(),
                    labels: HashMap::new(),
                    annotations: HashMap::new(),
                    runtime_handler: RuntimeHandler::WASI.to_string(),
                },
            );
        }
        let req = Request::new(RemovePodSandboxRequest {
            pod_sandbox_id: "1".to_owned(),
        });
        let res = svc.remove_pod_sandbox(req).await;
        // we expect an empty response object
        res.expect("remove sandbox result");
        // TODO(bacongobbler): un-comment this once remove_container() has been implemented.
        // assert_eq!(0, svc.containers.len());
        assert_eq!(
            0,
            svc.sandboxes
                .read()
                .unwrap()
                .values()
                .cloned()
                .collect::<Vec<PodSandbox>>()
                .len()
        );
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

    async fn _test_create_container() {
        let svc = CriRuntimeService::new();
        let req = Request::new(CreateContainerRequest::default());
        let res = svc.create_container(req).await;
        assert_eq!(
            "1",
            res.expect("successful create container")
                .get_ref()
                .container_id
        );
    }

    async fn _test_start_container() {
        let svc = CriRuntimeService::new();
        let req = Request::new(StartContainerRequest::default());
        let res = svc.start_container(req).await;
        // We expect an empty response object
        res.expect("start container result");
    }

    async fn _test_stop_container() {
        let svc = CriRuntimeService::new();
        let req = Request::new(StopContainerRequest::default());
        let res = svc.stop_container(req).await;
        // We expect an empty response object
        res.expect("stop container result");
    }

    async fn _test_remove_container() {
        let svc = CriRuntimeService::new();
        let req = Request::new(RemoveContainerRequest::default());
        let res = svc.remove_container(req).await;
        // We expect an empty response object
        res.expect("remove container result");
    }

    async fn _test_list_containers() {
        let svc = CriRuntimeService::new();
        let req = Request::new(ListContainersRequest::default());
        let res = svc.list_containers(req).await;
        assert_eq!(
            0,
            res.expect("successful list containers")
                .get_ref()
                .containers
                .len()
        );
    }

    async fn _test_container_status() {
        let svc = CriRuntimeService::new();
        let req = Request::new(ContainerStatusRequest::default());
        let res = svc.container_status(req).await;
        assert_eq!(
            "because I said so",
            res.expect("successful container status")
                .into_inner()
                .status
                .unwrap()
                .reason
        );
    }

    async fn _test_run_pod_sandbox() {
        let svc = CriRuntimeService::new();
        let mut sandbox_req = RunPodSandboxRequest::default();
        sandbox_req.runtime_handler = RuntimeHandler::WASI.to_string();

        // Create a temporary log dir for testing purposes
        let dir = tempdir().unwrap();
        let log_dir_name = dir.path().join("testdir");
        let mut conf = PodSandboxConfig::default();
        conf.log_directory = log_dir_name.to_str().unwrap().to_owned();
        sandbox_req.config = Some(conf);
        let req = Request::new(sandbox_req);
        let res = svc.run_pod_sandbox(req).await;
        // Make sure we receive back a valid UUID
        uuid::Uuid::parse_str(&res.unwrap().get_ref().pod_sandbox_id).unwrap();

        // Now check that the log directory was created
        assert_eq!(true, log_dir_name.exists());
    }

    async fn _test_create_and_list() {
        let svc = CriRuntimeService::new();
        let mut sandbox_req = RunPodSandboxRequest::default();
        sandbox_req.runtime_handler = RuntimeHandler::WASI.to_string();

        // Create a temporary log dir for testing purposes
        let dir = tempdir().unwrap();
        let log_dir_name = dir.path().join("testdir");
        let mut conf = PodSandboxConfig::default();
        conf.log_directory = log_dir_name.to_str().unwrap().to_owned();
        sandbox_req.config = Some(conf);
        let req = Request::new(sandbox_req);
        let res = svc.run_pod_sandbox(req).await;

        let id = res.unwrap().get_ref().pod_sandbox_id.clone();
        let list_req = Request::new(ListPodSandboxRequest::default());
        let res = svc.list_pod_sandbox(list_req).await;
        let sandboxes = res.expect("successful pod list").get_ref().items.clone();
        assert_eq!(1, sandboxes.len());
        // And make sure the UID returned actually exists
        assert_eq!(id, sandboxes[0].id);
    }
}
