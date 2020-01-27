use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use chrono::Utc;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::util;

// RuntimeService is converted to a package runtime_service_server
use crate::grpc::{
    runtime_service_server::RuntimeService, Container, ContainerConfig, ContainerMetadata,
    ContainerState, ContainerStatus, ContainerStatusRequest, ContainerStatusResponse,
    CreateContainerRequest, CreateContainerResponse, ImageSpec, ListContainersRequest,
    ListContainersResponse, ListPodSandboxRequest, ListPodSandboxResponse, Mount, PodSandbox,
    PodSandboxState, PodSandboxStatusRequest, PodSandboxStatusResponse, RemoveContainerRequest,
    RemoveContainerResponse, RemovePodSandboxRequest, RemovePodSandboxResponse,
    RunPodSandboxRequest, RunPodSandboxResponse, RuntimeCondition, RuntimeStatus,
    StartContainerRequest, StartContainerResponse, StatusRequest, StatusResponse,
    StopContainerRequest, StopContainerResponse, StopPodSandboxRequest, StopPodSandboxResponse,
    VersionRequest, VersionResponse,
};
use crate::wasm::Runtime;
use log::error;
use std::sync::mpsc::{channel, Sender};
use tokio::task::JoinHandle;

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

/// UserContainer is an internal mapping between the Container and the ContainerConfig objects provided by the kubelet.
/// We use this to map between what the CRI requested and what we created. (e.g. the volume mount mappings between
/// the container and the sandbox)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserContainer {
    /// the container ID.
    id: String,
    /// the pod sandbox ID this container belongs to.
    pod_sandbox_id: String,
    /// the resolved image reference.
    image_ref: String,
    /// the time this container was created, in nanoseconds.
    created_at: i64,
    /// the container's current state.
    state: i32,
    /// the CRI container config.
    config: ContainerConfig,
    /// Absolute path for the container to store the logs (STDOUT and STDERR) on the host.
    ///
    /// If the log_path is None, logging is disabled, either because the sandbox or the container did not specify a log path.
    log_path: Option<PathBuf>,
    /// volume paths for the container. host_path is a relative filepath from the container's root directory to the volume mount.
    /// container_path is the filepath specified from the container config's requested volume. This is used to map between the
    /// volume and the requested host_path/container_path.
    ///
    /// e.g.,
    ///     volumes = vec![Mount{container_path: "/app", host_path: "volumes/aaaa-bbbb-cccc-dddd", ...}]
    ///     config.mounts[0].container_path = "/app"
    ///     config.mounts[0].host_path = "/tmp/app"
    volumes: Vec<Mount>,
}

impl From<UserContainer> for Container {
    fn from(item: UserContainer) -> Self {
        Container {
            id: item.id,
            pod_sandbox_id: item.pod_sandbox_id,
            image: item.config.image,
            created_at: item.created_at,
            image_ref: item.image_ref,
            annotations: item.config.annotations,
            labels: item.config.labels,
            state: item.state,
            metadata: item.config.metadata,
        }
    }
}

/// Implement a CRI runtime service.
#[derive(Debug, Default)]
pub struct CriRuntimeService {
    // NOTE: we could replace this with evmap or crossbeam
    sandboxes: Arc<RwLock<BTreeMap<String, PodSandbox>>>,
    containers: Arc<RwLock<Vec<UserContainer>>>,
    root_dir: PathBuf,
}

impl CriRuntimeService {
    pub fn new(dir: PathBuf) -> Self {
        util::ensure_root_dir(&dir).expect("cannot create root directory for runtime service");
        CriRuntimeService {
            sandboxes: Arc::new(RwLock::new(BTreeMap::default())),
            containers: Arc::new(RwLock::new(vec![])),
            root_dir: dir,
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

    async fn status(&self, req: Request<StatusRequest>) -> CriResult<StatusResponse> {
        let mut extra_info = HashMap::new();
        if req.into_inner().verbose {
            extra_info.insert(
                "running_sandboxes".to_owned(),
                self.sandboxes.read().unwrap().len().to_string(),
            );
            extra_info.insert(
                "running_containers".to_owned(),
                self.containers.read().unwrap().len().to_string(),
            );
        }

        Ok(Response::new(StatusResponse {
            status: Some(RuntimeStatus {
                conditions: vec![
                    // There isn't anything else to change on these right now,
                    // so keep them hard coded. If we start needing to update
                    // these (such as with networking) or add our own arbitrary
                    // conditions, we can move them into the struct
                    RuntimeCondition {
                        r#type: "RuntimeReady".to_owned(),
                        status: true,
                        // NOTE: We should make these reasons an enum once we
                        // actually define more of them
                        reason: "RuntimeStarted".to_owned(),
                        message: "Runtime has been started and is ready to run modules".to_owned(),
                    },
                    RuntimeCondition {
                        r#type: "NetworkReady".to_owned(),
                        status: false, // False until we figure out networking support
                        reason: "Unimplemented".to_owned(),
                        message: "Networking is currently unimplemented".to_owned(),
                    },
                ],
            }),
            info: extra_info,
        }))
    }

    async fn run_pod_sandbox(
        &self,
        req: Request<RunPodSandboxRequest>,
    ) -> CriResult<RunPodSandboxResponse> {
        let sandbox_req = req.into_inner();
        let sandbox_conf = sandbox_req
            .config
            .ok_or_else(|| Status::invalid_argument("Sandbox request is missing config object"))?;
        let handler = RuntimeHandler::from_string(&sandbox_req.runtime_handler)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        // TODO(taylor): As of now, there isn't networking support in wasmtime,
        // so we can't necessarily set it up right now. Once it does, we'll need
        // to set up networking here

        // Create the logs directory for this pod
        tokio::fs::create_dir_all(&sandbox_conf.log_directory).await?;
        // Basically, everything above here is all we need to set up a sandbox.
        // All of the security context stuff pretty much doesn't matter for
        // WASM, but we can revisit this as things keep evolving

        // TODO(taylor): According to the RwLock docs, we should panic on failure
        // to poison the RwLock. This also means we'd need to have handling for
        // recovering from a poisoned RwLock, which I am leaving for later
        let mut sandboxes = self
            .sandboxes
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
        req: Request<StopPodSandboxRequest>,
    ) -> CriResult<StopPodSandboxResponse> {
        let id = req.into_inner().pod_sandbox_id;

        let mut sandboxes = self.sandboxes.write().unwrap();
        let mut sandbox = match sandboxes.get_mut(&id) {
            Some(s) => s,
            None => return Err(Status::not_found(format!("Sandbox {} does not exist", id))),
        };

        // Stop all containers inside the sandbox. This forcibly terminates all containers with no grace period.
        let container_store = self.containers.read().unwrap();
        let containers: Vec<&UserContainer> = container_store.iter().filter(|x| x.pod_sandbox_id == id).collect();
        for container in containers {
            self.stop_container(Request::new(StopContainerRequest {
                container_id: container.id.clone(),
                timeout: 0,
            }));
        }

        // mark the pod sandbox as not ready, preventing future container creation.
        sandbox.state = PodSandboxState::SandboxNotready as i32;

        // TODO(bacongobbler): when networking is implemented, here is where we should tear down the network.

        Ok(Response::new(StopPodSandboxResponse {}))
    }

    // remove_pod_sandbox removes the sandbox. If there are running containers in the sandbox, they should be forcibly
    // removed.
    async fn remove_pod_sandbox(
        &self,
        req: Request<RemovePodSandboxRequest>,
    ) -> CriResult<RemovePodSandboxResponse> {
        let id = &req.into_inner().pod_sandbox_id;

        let mut sandboxes = self.sandboxes.write().unwrap();
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

        // TODO(bacongobbler): when networking is implemented, here is where we should return an error if the sandbox's
        // network namespace is not closed yet.

        // remove all containers inside the sandbox.
        for container in self.containers.read().unwrap().iter() {
            if &container.pod_sandbox_id == id {
                self.remove_container(Request::new(RemoveContainerRequest {
                    container_id: container.id.clone(),
                }));
            }
        }

        // remove the sandbox.
        sandboxes.remove(id);

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
        req: Request<CreateContainerRequest>,
    ) -> CriResult<CreateContainerResponse> {
        let container_req = req.into_inner();
        let container_config = container_req.config.unwrap_or_default();
        let sandbox_config = container_req.sandbox_config.unwrap_or_default();

        // generate a unique ID for the container
        //
        // TODO(bacongobbler): we should probably commit this to a RWLock'd map; that way concurrent calls to
        // create_container() won't reserve the same name. Fow now, let's just generate a "unique enough" UUID.
        //
        // https://github.com/containerd/cri/blob/b2804c06934245b0ff4a9114c9f1f592a5120815/pkg/server/container_create.go#L63-L81
        let id = Uuid::new_v4().to_string();

        let mut container = UserContainer {
            id: id.to_owned(),
            pod_sandbox_id: container_req.pod_sandbox_id,
            state: ContainerState::ContainerCreated as i32,
            created_at: Utc::now().timestamp_nanos(),
            config: container_config.to_owned(),
            log_path: None,           // to be set further down
            image_ref: "".to_owned(), // FIXME(bacongobbler): resolve this to the local image reference based on config.image.name
            volumes: vec![],          // to be added further down
        };

        // create container root directory.
        let container_root_dir = self.root_dir.join("containers").join(&id);
        std::fs::create_dir_all(&container_root_dir)?;

        // generate volume mounts.
        for mount in container_config.mounts {
            let volume_id = Uuid::new_v4().to_string();
            container.volumes.push(Mount {
                host_path: PathBuf::from("volumes")
                    .join(volume_id)
                    .into_os_string()
                    .into_string()
                    .unwrap(),
                container_path: mount.container_path.to_owned(),
                propagation: mount.propagation,
                readonly: mount.readonly,
                selinux_relabel: mount.selinux_relabel,
            })
        }

        // validate log paths and compose full container log path.
        if sandbox_config.log_directory != "" && container.config.log_path != "" {
            container.log_path =
                Some(PathBuf::from(&sandbox_config.log_directory).join(&container.config.log_path));
            log::debug!("composed container log path using sandbox log directory {} and container config log path {}", sandbox_config.log_directory, container.config.log_path);
        } else {
            // logging is disabled
            log::info!(
                "logging will be disabled due to empty log paths for sandbox {} or container {}",
                sandbox_config.log_directory,
                container.config.log_path
            );
        }

        // add container to the store.
        self.containers.write().unwrap().push(container);

        Ok(Response::new(CreateContainerResponse { container_id: id }))
    }

    async fn start_container(
        &self,
        req: Request<StartContainerRequest>,
    ) -> CriResult<StartContainerResponse> {
        let id = req.into_inner().container_id;
        let containers = self.containers.read().unwrap();
        let container = containers
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| Status::not_found("Container not found"))?;

        let sandboxes = self.sandboxes.read().unwrap();
        let sandbox = sandboxes
            .get(&container.pod_sandbox_id)
            .ok_or_else(|| Status::not_found("Sandbox not found"))?;

        let runtime = RuntimeHandler::from_string(&sandbox.runtime_handler)
            .map_err(|_| Status::invalid_argument("Invalid runtime handler"))?;
        match runtime {
            RuntimeHandler::WASCC => unimplemented!(),
            RuntimeHandler::WASI => {
                use std::convert::TryFrom;
                let image_ref = crate::oci::ImageRef::try_from(&container.image_ref).unwrap();
                let path = image_ref
                    .file_path(&crate::oci::default_image_dir())
                    .into_os_string()
                    .into_string()
                    .unwrap();
                let runtime = crate::wasm::WasiRuntime::new(
                    path,
                    HashMap::new(),
                    container.config.args.clone(),
                    HashMap::new(),
                    &container.log_path.as_ref().unwrap_or(&PathBuf::new()),
                )
                .unwrap();
                RuntimeContainer::new(runtime).start();
            }
        };
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
            containers: self
                .containers
                .read()
                .unwrap()
                .iter()
                .cloned()
                .map(Container::from)
                .collect(),
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
    use tempfile::tempdir;
    use tonic::Request;

    #[tokio::test]
    async fn test_version() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
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

    #[tokio::test]
    async fn test_status() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
        let res = svc
            .status(Request::new(StatusRequest::default()))
            .await
            .expect("successful status request");
        assert_eq!(
            res.get_ref().status.as_ref().unwrap().conditions,
            vec![
                // There isn't anything else to change on these right now,
                // so keep them hard coded. If we start needing to update
                // these (such as with networking) or add our own arbitrary
                // conditions, we can move them into the struct
                RuntimeCondition {
                    r#type: "RuntimeReady".to_owned(),
                    status: true,
                    // NOTE: We should make these reasons an enum once we
                    // actually define more of them
                    reason: "RuntimeStarted".to_owned(),
                    message: "Runtime has been started and is ready to run modules".to_owned(),
                },
                RuntimeCondition {
                    r#type: "NetworkReady".to_owned(),
                    status: false, // False until we figure out networking support
                    reason: "Unimplemented".to_owned(),
                    message: "Networking is currently unimplemented".to_owned(),
                },
            ]
        );

        // Make sure info is empty because it wasn't verbose
        assert!(res.get_ref().info.is_empty());

        // now double check that info gets data if verbose is requested
        let mut req = StatusRequest::default();
        req.verbose = true;
        let res = svc
            .status(Request::new(req))
            .await
            .expect("successful status request");
        let info = &res.get_ref().info;
        assert!(!info.is_empty());
        assert!(info.contains_key("running_sandboxes"));
        assert!(info.contains_key("running_containers"));
    }

    #[tokio::test]
    async fn test_list_pod_sandbox() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
        let req = Request::new(ListPodSandboxRequest::default());
        let res = svc.list_pod_sandbox(req).await;
        assert_eq!(0, res.expect("successful pod list").get_ref().items.len());
    }

    #[tokio::test]
    async fn test_pod_sandbox_status() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
        let req = Request::new(PodSandboxStatusRequest::default());
        let res = svc.pod_sandbox_status(req).await;
        assert_eq!(None, res.expect("status result").get_ref().status);
    }

    #[tokio::test]
    async fn test_remove_pod_sandbox() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
        let mut container = UserContainer::default();
        container.pod_sandbox_id = "1".to_owned();
        svc.containers.write().unwrap().push(container);

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
        drop(sandboxes);
        let req = Request::new(RemovePodSandboxRequest {
            pod_sandbox_id: "1".to_owned(),
        });
        let res = svc.remove_pod_sandbox(req).await;
        // we expect an empty response object
        res.expect("remove sandbox result");
        // TODO(bacongobbler): un-comment this once remove_container() has been implemented.
        // assert_eq!(0, svc.containers.len());
        assert_eq!(0, svc.sandboxes.read().unwrap().values().cloned().len());
    }

    #[tokio::test]
    async fn test_stop_pod_sandbox() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
        let mut sandboxes = svc.sandboxes.write().unwrap();
        sandboxes.insert(
            "test".to_owned(),
            PodSandbox {
                id: "test".to_owned(),
                metadata: None,
                state: PodSandboxState::SandboxReady as i32,
                created_at: Utc::now().timestamp_nanos(),
                labels: HashMap::new(),
                annotations: HashMap::new(),
                runtime_handler: RuntimeHandler::WASI.to_string(),
            },
        );
        drop(sandboxes);
        let req = Request::new(StopPodSandboxRequest {
            pod_sandbox_id: "test".to_owned(),
        });
        let res = svc.stop_pod_sandbox(req).await;

        // Expect the stopped ID to be the same as the requested ID.
        res.expect("empty stop result");

        // test what happens when the requested pod sandbox doesn't exist
        let req = Request::new(StopPodSandboxRequest {
            pod_sandbox_id: "doesnotexist".to_owned(),
        });
        let res = svc.stop_pod_sandbox(req).await;
        res.expect_err("pod sandbox does not exist");
    }

    #[tokio::test]
    async fn test_create_container() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
        let mut sandboxes = svc.sandboxes.write().unwrap();
        sandboxes.insert(
            "test".to_owned(),
            PodSandbox {
                id: "test".to_owned(),
                metadata: None,
                state: PodSandboxState::SandboxReady as i32,
                created_at: Utc::now().timestamp_nanos(),
                labels: HashMap::new(),
                annotations: HashMap::new(),
                runtime_handler: RuntimeHandler::WASI.to_string(),
            },
        );
        drop(sandboxes);
        let req = Request::new(CreateContainerRequest{
            pod_sandbox_id: "test".to_owned(),
            config: None,
            sandbox_config: None
        });
        let res = svc.create_container(req).await;
        // We can't have a deterministic container id, so just check it is a valid uuid
        uuid::Uuid::parse_str(
            &res.expect("successful create container")
                .get_ref()
                .container_id,
        )
        .unwrap();
        assert_eq!(1, svc.containers.read().unwrap().len());
    }

    #[tokio::test]
    async fn test_start_container() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
        let req = Request::new(StartContainerRequest::default());
        let res = svc.start_container(req).await;
        // We expect an empty response object
        res.expect("start container result");
    }

    #[tokio::test]
    async fn test_stop_container() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
        let req = Request::new(StopContainerRequest::default());
        let res = svc.stop_container(req).await;
        // We expect an empty response object
        res.expect("stop container result");
    }

    #[tokio::test]
    async fn test_remove_container() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
        let req = Request::new(RemoveContainerRequest::default());
        let res = svc.remove_container(req).await;
        // We expect an empty response object
        res.expect("remove container result");
    }

    #[tokio::test]
    async fn test_list_containers() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
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

    #[tokio::test]
    async fn test_container_status() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
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

    #[tokio::test]
    async fn test_run_pod_sandbox() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
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

    #[tokio::test]
    async fn test_create_and_list() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
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

pub struct RuntimeContainer {
    handle: JoinHandle<Result<()>>,
    sender: Sender<()>,
}

impl RuntimeContainer {
    pub fn new<T: Runtime + Send + 'static>(rt: T) -> Self {
        let (sender, receiver) = channel::<()>();
        let handle = tokio::task::spawn_blocking(move || {
            let _ = receiver.recv().unwrap();
            if let Err(e) = rt.run() {
                // TODO(taylor): Implement messaging here to indicate that there was a problem running the module
                error!("Error while running module: {}", e);
                todo!("Error type")
            }
            Ok(())
        });
        RuntimeContainer { handle, sender }
    }

    pub fn start(self) -> RuntimeContainerCancellationToken {
        self.sender.send(()).unwrap();
        RuntimeContainerCancellationToken(self.handle)
    }
}

pub struct RuntimeContainerCancellationToken(JoinHandle<Result<()>>);

impl RuntimeContainerCancellationToken {
    pub fn stop(self) {
        panic!("Stopping a running container is not currently supported");
    }
}
