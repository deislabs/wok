use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use chrono::Utc;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::util;

// RuntimeService is converted to a package runtime_service_server
use crate::grpc::{
    runtime_service_server::RuntimeService, Container, ContainerAttributes, ContainerConfig,
    ContainerMetadata, ContainerState, ContainerStats, ContainerStatsRequest,
    ContainerStatsResponse, ContainerStatus, ContainerStatusRequest, ContainerStatusResponse,
    CreateContainerRequest, CreateContainerResponse, ImageSpec, ListContainerStatsRequest,
    ListContainerStatsResponse, ListContainersRequest, ListContainersResponse,
    ListPodSandboxRequest, ListPodSandboxResponse, Mount, PodSandbox, PodSandboxState,
    PodSandboxStatus, PodSandboxStatusRequest, PodSandboxStatusResponse, RemoveContainerRequest,
    RemoveContainerResponse, RemovePodSandboxRequest, RemovePodSandboxResponse,
    RunPodSandboxRequest, RunPodSandboxResponse, RuntimeCondition, RuntimeStatus,
    StartContainerRequest, StartContainerResponse, StatusRequest, StatusResponse,
    StopContainerRequest, StopContainerResponse, StopPodSandboxRequest, StopPodSandboxResponse,
    VersionRequest, VersionResponse,
};
use crate::wasm::{LucetRuntime, Runtime};

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

impl From<UserContainer> for ContainerStats {
    fn from(item: UserContainer) -> Self {
        ContainerStats {
            attributes: Some(ContainerAttributes {
                id: item.id,
                metadata: item.config.metadata,
                labels: item.config.labels,
                annotations: item.config.annotations,
            }),
            // TODO(taylor): Fetch this and memory usage from the running
            // thread? If so, we can't use the From trait because we'd need to
            // handle errors from trying to get the data
            cpu: None,
            memory: None,
            // We don't have an attached filesystem like containers do, so this
            // shouldn't matter.
            writable_layer: None,
        }
    }
}

impl From<PodSandbox> for PodSandboxStatus {
    fn from(item: PodSandbox) -> Self {
        PodSandboxStatus {
            id: item.id,
            metadata: item.metadata,
            created_at: item.created_at,
            annotations: item.annotations,
            labels: item.labels,
            state: item.state,
            runtime_handler: item.runtime_handler,
            network: None, // to be populated by the caller
            linux: None,   // unused by wok
        }
    }
}

/// Implement a CRI runtime service.
#[derive(Default)]
pub struct CriRuntimeService {
    // NOTE: we could replace this with evmap or crossbeam
    sandboxes: Arc<RwLock<BTreeMap<String, PodSandbox>>>,
    containers: Arc<RwLock<Vec<UserContainer>>>,
    running_containers: Arc<RwLock<HashMap<String, LucetRuntime>>>,
    root_dir: PathBuf,
}

impl CriRuntimeService {
    pub fn new(dir: PathBuf) -> Self {
        util::ensure_root_dir(&dir).expect("cannot create root directory for runtime service");
        CriRuntimeService {
            sandboxes: Arc::new(RwLock::new(BTreeMap::default())),
            containers: Arc::new(RwLock::new(vec![])),
            running_containers: Arc::new(RwLock::new(HashMap::new())),
            root_dir: dir,
        }
    }
}

#[derive(Debug)]
pub enum RuntimeHandler {
    WASI,
    WASCC,
    LUCET,
}

impl ToString for RuntimeHandler {
    fn to_string(&self) -> String {
        match self {
            Self::WASI => "WASI".to_owned(),
            Self::WASCC => "WASCC".to_owned(),
            Self::LUCET => "LUCET".to_owned(),
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
            "LUCET" => Ok(Self::LUCET),
            _ => Err(format_err!("Invalid runtime handler {}", s)),
        }
    }
}

impl Default for RuntimeHandler {
    fn default() -> Self {
        Self::LUCET
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
        let containers: Vec<&UserContainer> = container_store
            .iter()
            .filter(|x| x.pod_sandbox_id == id)
            .collect();
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
        req: Request<PodSandboxStatusRequest>,
    ) -> CriResult<PodSandboxStatusResponse> {
        let request = req.into_inner();

        let sandboxes = self.sandboxes.read().unwrap();
        let sandbox = match sandboxes.get(&request.pod_sandbox_id) {
            Some(s) => s,
            None => {
                return Err(Status::not_found(format!(
                    "Sandbox {} does not exist",
                    request.pod_sandbox_id
                )))
            }
        };

        let status = PodSandboxStatus::from(sandbox.clone());

        // TODO(bacongobbler): report back status on the network and linux-specific sandbox status here (when implemented)

        // TODO: generate any verbose information we want to report if requested.
        //
        // Right now, we don't have any extra information to provide to the user here, so we'll just return what we know.

        Ok(Response::new(PodSandboxStatusResponse {
            info: HashMap::new(),
            status: Some(status),
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
            log_path: None, // to be set further down
            image_ref: container_config.image.as_ref().unwrap().image.clone(), // FIXME(rylev): understand what it means for the image to be None
            volumes: vec![], // to be added further down
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

        // Create specific scope for the container read lock
        let (runtime, container) = {
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

            (runtime, container.clone())
        };
        // TODO: handle unset log path
        let log_path = container.log_path.as_ref().expect("No log set");
        tokio::fs::create_dir_all(log_path).await?;

        match runtime {
            RuntimeHandler::WASCC => todo!("Handle wasCC"),
            RuntimeHandler::WASI => todo!("Handle WASI"),
            // RuntimeHandler::WASI => {
            //     use std::convert::TryFrom;

            //     // TODO: handle error
            //     let image_ref = crate::oci::ImageRef::try_from(&container.image_ref)
            //         .expect("Failed to parse image_ref");
            //     let module_path = image_ref
            //         .file_path(&self.root_dir)
            //         .into_os_string()
            //         .into_string()
            //         .unwrap();
            //     let env: HashMap<String, String> = container
            //         .config
            //         .envs
            //         .iter()
            //         .cloned()
            //         .map(|pair| (pair.key, pair.value))
            //         .collect();

            //     println!("Reading {:?}", module_path);

            //     let runtime = WasiRuntime::new(
            //         module_path,
            //         env,
            //         container.config.args.clone(),
            //         // TODO: dirs
            //         HashMap::new(),
            //         &log_path,
            //     )
            //     .expect("Creating runtime failed");

            //     runtime.run();
            //     let mut running_containers = self.running_containers.write().unwrap();
            //     running_containers.insert(container.id, runtime);
            // }
            RuntimeHandler::LUCET => {
                use std::convert::TryFrom;

                // TODO: handle error
                let image_ref = crate::oci::ImageRef::try_from(&container.image_ref)
                    .expect("Failed to parse image_ref");
                let module_path = image_ref
                    .file_path(&self.root_dir)
                    .into_os_string()
                    .into_string()
                    .unwrap();
                let env: HashMap<String, String> = container
                    .config
                    .envs
                    .iter()
                    .cloned()
                    .map(|pair| (pair.key, pair.value))
                    .collect();

                let mut runtime = LucetRuntime::new(
                    PathBuf::from(module_path),
                    env,
                    container.config.args.clone(),
                    // TODO: dirs
                    HashMap::new(),
                    &log_path,
                )
                .expect("Lucet runtime can't be created");

                runtime.run().expect("Lucet runtime can't run");
                let mut running_containers = self.running_containers.write().unwrap();
                running_containers.insert(container.id, runtime);
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
        req: Request<RemoveContainerRequest>,
    ) -> CriResult<RemoveContainerResponse> {
        let id = req.into_inner().container_id;
        // TODO(taylor): Force a container stop once we have a method for
        // handling it
        self.containers.write().unwrap().retain(|c| c.id != id);
        Ok(Response::new(RemoveContainerResponse {}))
    }

    async fn list_containers(
        &self,
        req: Request<ListContainersRequest>,
    ) -> CriResult<ListContainersResponse> {
        let filter: crate::grpc::ContainerFilter = req.into_inner().filter.unwrap_or_default();
        Ok(Response::new(ListContainersResponse {
            containers: self
                .containers
                .read()
                .unwrap()
                .iter()
                .cloned()
                .filter(|c| {
                    (filter.id == "" || c.id == filter.id)
                        && filter
                            .state
                            .as_ref()
                            .map(|s| s.state == c.state)
                            .unwrap_or(true)
                        && (filter.pod_sandbox_id == ""
                            || c.pod_sandbox_id == filter.pod_sandbox_id)
                        && (filter.label_selector.is_empty()
                            || has_labels(&filter.label_selector, &c.config.labels))
                })
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

    async fn container_stats(
        &self,
        req: Request<ContainerStatsRequest>,
    ) -> CriResult<ContainerStatsResponse> {
        let id = req.into_inner().container_id;
        let containers = self.containers.read().unwrap();
        let container = containers
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| Status::not_found("Container not found"))?;
        Ok(Response::new(ContainerStatsResponse {
            stats: Some(container.clone().into()),
        }))
    }

    async fn list_container_stats(
        &self,
        req: Request<ListContainerStatsRequest>,
    ) -> CriResult<ListContainerStatsResponse> {
        let filter = req.into_inner().filter.unwrap_or_default();
        let containers = self.containers.read().unwrap();
        let container_stats: Vec<ContainerStats> = containers
            .iter()
            .filter(|c| {
                (filter.id == "" || c.id == filter.id)
                    && (filter.pod_sandbox_id == "" || c.pod_sandbox_id == filter.pod_sandbox_id)
                    && (filter.label_selector.is_empty()
                        || has_labels(&filter.label_selector, &c.config.labels))
            })
            .cloned()
            .map(ContainerStats::from)
            .collect();
        Ok(Response::new(ListContainerStatsResponse {
            stats: container_stats,
        }))
    }
}

// For use in checking if label maps (or any String, String maps) contain all of
// the search labels (an AND query)
pub(crate) fn has_labels(
    search_labels: &HashMap<String, String>,
    target_labels: &HashMap<String, String>,
) -> bool {
    for (key, val) in search_labels.iter() {
        if target_labels.get(key) != Some(val) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::grpc::*;
    use tempfile::tempdir;
    use tonic::Request;

    #[test]
    fn test_has_labels() {
        let mut search_labels = HashMap::new();
        let mut target_labels = HashMap::new();

        target_labels.insert("foo".to_owned(), "bar".to_owned());
        target_labels.insert("blah".to_owned(), "blah".to_owned());

        // Empty search should return true
        assert!(has_labels(&search_labels, &target_labels));

        // Missing search term in target should be false
        search_labels.insert("notreal".to_owned(), "a non existent value".to_owned());
        search_labels.insert("foo".to_owned(), "bar".to_owned());
        search_labels.insert("blah".to_owned(), "blah".to_owned());
        assert!(!has_labels(&search_labels, &target_labels));

        // Matching search should return true
        search_labels.remove("notreal");
        assert!(has_labels(&search_labels, &target_labels));
    }

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
        let mut sandboxes = svc.sandboxes.write().unwrap();
        let sandbox = PodSandbox {
            id: "1".to_owned(),
            metadata: None,
            state: PodSandboxState::SandboxNotready as i32,
            created_at: Utc::now().timestamp_nanos(),
            labels: HashMap::new(),
            annotations: HashMap::new(),
            runtime_handler: RuntimeHandler::WASI.to_string(),
        };
        sandboxes.insert(sandbox.id.clone(), sandbox);
        drop(sandboxes);
        let req = Request::new(PodSandboxStatusRequest {
            pod_sandbox_id: "1".to_owned(),
            verbose: true,
        });
        let res = svc.pod_sandbox_status(req).await;
        assert_eq!(
            "1",
            res.expect("status result")
                .get_ref()
                .status
                .as_ref()
                .expect("status result")
                .id
        );
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
        let mut config = ContainerConfig::default();
        config.image = Some(ImageSpec {
            image: "foo/bar:baz".to_owned(),
        });
        let req = Request::new(CreateContainerRequest {
            pod_sandbox_id: "test".to_owned(),
            config: Some(config),
            sandbox_config: None,
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
        // Put every file in a temp dir so it's automatically cleaned up
        let dir = tempdir().expect("Couldn't create temp directory");
        let svc = CriRuntimeService::new(dir.path().to_owned());

        let image_ref = crate::oci::ImageRef {
            whole: "foo/bar:baz",
            registry: "foo",
            repo: "bar",
            tag: "baz",
        };

        // create temp directories
        let log_dir_name = dir.path().join("testdir");
        let image_file = image_ref.file_path(dir.path());
        // log directory
        tokio::fs::create_dir_all(&log_dir_name)
            .await
            .expect("Could't create log directory");
        // wasm file directory
        tokio::fs::create_dir_all(image_file.parent().unwrap())
            .await
            .expect("Couldn't create wasm file director");
        // read and write wasm
        let wasm = tokio::fs::read("examples/printer.wasm")
            .await
            .expect("Couldn't read wasm");
        tokio::fs::write(image_file, wasm)
            .await
            .expect("couldn't write wasm");

        // create sandbox and container
        let sandbox = PodSandbox::default();
        let mut container = UserContainer::default();

        let container_id = {
            // write container
            let mut containers = svc.containers.write().unwrap();
            container.pod_sandbox_id = sandbox.id.clone();
            container.log_path = Some(log_dir_name);
            container.image_ref = image_ref.whole.to_owned();
            let container_id = container.id.clone();

            containers.push(container);

            // write sandbox
            let mut sandboxes = svc.sandboxes.write().unwrap();
            sandboxes.insert(sandbox.id.clone(), sandbox);
            container_id
        };
        let req = Request::new(StartContainerRequest { container_id });
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
        let mut containers = svc.containers.write().unwrap();
        containers.push(UserContainer {
            id: "test".to_owned(),
            pod_sandbox_id: "test".to_owned(),
            image_ref: "doesntmatter".to_owned(),
            created_at: Utc::now().timestamp_nanos(),
            state: ContainerState::ContainerRunning as i32,
            config: ContainerConfig::default(),
            log_path: None,
            volumes: Vec::default(),
        });
        containers.push(UserContainer {
            id: "foo".to_owned(),
            pod_sandbox_id: "foo".to_owned(),
            image_ref: "doesntmatter".to_owned(),
            created_at: Utc::now().timestamp_nanos(),
            state: ContainerState::ContainerRunning as i32,
            config: ContainerConfig::default(),
            log_path: None,
            volumes: Vec::default(),
        });
        drop(containers);
        let req = Request::new(RemoveContainerRequest {
            container_id: "test".to_owned(),
        });
        let res = svc.remove_container(req).await;
        // We expect an empty response object
        res.expect("remove container result");
        // Check for the container to be gone and that we still have one left
        assert_eq!(1, svc.containers.read().unwrap().len());
    }

    #[tokio::test]
    async fn test_list_containers() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
        let mut containers = svc.containers.write().unwrap();
        let mut labels = HashMap::new();
        labels.insert("test1".to_owned(), "testing".to_owned());
        containers.push(UserContainer {
            id: "test".to_owned(),
            pod_sandbox_id: "test".to_owned(),
            state: ContainerState::ContainerRunning as i32,
            config: ContainerConfig {
                metadata: Some(ContainerMetadata {
                    attempt: 1,
                    name: "test".to_owned(),
                }),
                labels: labels.clone(),
                ..Default::default()
            },
            ..Default::default()
        });
        containers.push(UserContainer {
            id: "test2".to_owned(),
            pod_sandbox_id: "test2".to_owned(),
            state: ContainerState::ContainerRunning as i32,
            config: ContainerConfig {
                metadata: Some(ContainerMetadata {
                    attempt: 1,
                    name: "test2".to_owned(),
                }),
                labels: HashMap::default(),
                ..Default::default()
            },
            ..Default::default()
        });
        containers.push(UserContainer {
            id: "test3".to_owned(),
            pod_sandbox_id: "test2".to_owned(),
            state: ContainerState::ContainerCreated as i32,
            config: ContainerConfig {
                metadata: Some(ContainerMetadata {
                    attempt: 1,
                    name: "test2".to_owned(),
                }),
                labels: HashMap::default(),
                ..Default::default()
            },
            ..Default::default()
        });
        drop(containers);
        let req = Request::new(ListContainersRequest {
            filter: Some(ContainerFilter::default()),
        });
        let res = svc.list_containers(req).await;
        // Nothing set should return all
        let containers = res.expect("list containers result").into_inner().containers;
        assert_eq!(3, containers.len());

        // Pod sandbox ID should return all containers in the given sandbox
        let req = Request::new(ListContainersRequest {
            filter: Some(ContainerFilter {
                pod_sandbox_id: "test2".to_owned(),
                ..Default::default()
            }),
        });
        let res = svc.list_containers(req).await;
        let containers = res.expect("list containers result").into_inner().containers;
        assert_eq!(2, containers.len());

        // Labels should only return matching containers
        let req = Request::new(ListContainersRequest {
            filter: Some(ContainerFilter {
                label_selector: labels,
                ..Default::default()
            }),
        });
        let res = svc.list_containers(req).await;
        let containers = res.expect("list containers result").into_inner().containers;
        assert_eq!(1, containers.len());

        // ID and sandbox ID should return a specific container
        let req = Request::new(ListContainersRequest {
            filter: Some(ContainerFilter {
                id: "test2".to_owned(),
                pod_sandbox_id: "test2".to_owned(),
                ..Default::default()
            }),
        });
        let res = svc.list_containers(req).await;
        let containers = res.expect("list containers result").into_inner().containers;
        assert_eq!(1, containers.len());

        // Status should return a specific container
        let req = Request::new(ListContainersRequest {
            filter: Some(ContainerFilter {
                state: Some(ContainerStateValue {
                    state: ContainerState::ContainerCreated as i32,
                }),
                ..Default::default()
            }),
        });
        let res = svc.list_containers(req).await;
        let containers = res.expect("list containers result").into_inner().containers;
        assert_eq!(1, containers.len());
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
    async fn test_container_stats() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
        let mut containers = svc.containers.write().unwrap();
        let mut labels = HashMap::new();
        labels.insert("test1".to_owned(), "testing".to_owned());
        containers.push(UserContainer {
            id: "test".to_owned(),
            pod_sandbox_id: "test".to_owned(),
            config: ContainerConfig {
                metadata: Some(ContainerMetadata {
                    attempt: 1,
                    name: "test".to_owned(),
                }),
                labels: labels.clone(),
                ..Default::default()
            },
            ..Default::default()
        });
        drop(containers);
        let req = Request::new(ContainerStatsRequest {
            container_id: "test".to_owned(),
        });
        let res = svc.container_stats(req).await;
        // We expect an empty response object
        let stats = res
            .expect("remove container result")
            .into_inner()
            .stats
            .unwrap();
        assert_eq!(
            stats,
            ContainerStats {
                attributes: Some(ContainerAttributes {
                    id: "test".to_owned(),
                    metadata: Some(ContainerMetadata {
                        attempt: 1,
                        name: "test".to_owned(),
                    }),
                    labels,
                    annotations: HashMap::new(),
                }),
                cpu: None,
                memory: None,
                writable_layer: None,
            }
        );
    }

    #[tokio::test]
    async fn test_list_container_stats() {
        let svc = CriRuntimeService::new(PathBuf::from(""));
        let mut containers = svc.containers.write().unwrap();
        let mut labels = HashMap::new();
        labels.insert("test1".to_owned(), "testing".to_owned());
        containers.push(UserContainer {
            id: "test".to_owned(),
            pod_sandbox_id: "test".to_owned(),
            config: ContainerConfig {
                metadata: Some(ContainerMetadata {
                    attempt: 1,
                    name: "test".to_owned(),
                }),
                labels: labels.clone(),
                ..Default::default()
            },
            ..Default::default()
        });
        containers.push(UserContainer {
            id: "test2".to_owned(),
            pod_sandbox_id: "test2".to_owned(),
            config: ContainerConfig {
                metadata: Some(ContainerMetadata {
                    attempt: 1,
                    name: "test2".to_owned(),
                }),
                labels: HashMap::default(),
                ..Default::default()
            },
            ..Default::default()
        });
        containers.push(UserContainer {
            id: "test3".to_owned(),
            pod_sandbox_id: "test2".to_owned(),
            config: ContainerConfig {
                metadata: Some(ContainerMetadata {
                    attempt: 1,
                    name: "test2".to_owned(),
                }),
                labels: HashMap::default(),
                ..Default::default()
            },
            ..Default::default()
        });
        drop(containers);
        let req = Request::new(ListContainerStatsRequest {
            filter: Some(ContainerStatsFilter::default()),
        });
        let res = svc.list_container_stats(req).await;
        // Nothing set should return all
        let stats = res.expect("list container stats result").into_inner().stats;
        assert_eq!(3, stats.len());

        // Pod sandbox ID should return all containers in the given sandbox
        let req = Request::new(ListContainerStatsRequest {
            filter: Some(ContainerStatsFilter {
                pod_sandbox_id: "test2".to_owned(),
                ..Default::default()
            }),
        });
        let res = svc.list_container_stats(req).await;
        let stats = res.expect("list container stats result").into_inner().stats;
        assert_eq!(2, stats.len());

        // Labels should only return matching containers
        let req = Request::new(ListContainerStatsRequest {
            filter: Some(ContainerStatsFilter {
                label_selector: labels,
                ..Default::default()
            }),
        });
        let res = svc.list_container_stats(req).await;
        let stats = res.expect("list container stats result").into_inner().stats;
        assert_eq!(1, stats.len());

        // ID and sandbox ID should return a specific container
        let req = Request::new(ListContainerStatsRequest {
            filter: Some(ContainerStatsFilter {
                id: "test2".to_owned(),
                pod_sandbox_id: "test2".to_owned(),
                ..Default::default()
            }),
        });
        let res = svc.list_container_stats(req).await;
        let stats = res.expect("list container stats result").into_inner().stats;
        assert_eq!(1, stats.len());
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
