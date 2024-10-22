use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;
use std::path::PathBuf;
use std::str::FromStr;

use chrono::Utc;
use ipnet::IpNet;
use log::{error, info};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tonic::{Request, Response, Status};
use uuid::Uuid;

// RuntimeService is converted to a package runtime_service_server
use super::grpc::{self, runtime_service_server::RuntimeService};
use super::CriResult;
use crate::docker::Reference;
use crate::store::ModuleStore;
use crate::wasm::wascc::*;
use crate::wasm::{Result, Runtime};

/// The version of the runtime API that this tool knows.
/// See CRI-O for reference (since docs don't explain this)
/// https://github.com/cri-o/cri-o/blob/master/server/version.go
const RUNTIME_API_VERSION: &str = "v1alpha2";
/// The API version of this CRI plugin.
const API_VERSION: &str = "0.1.0";

/// The Actor's public key is a required annotation on a container that runs a waSCC actor.
///
/// The key is used to verify that the WASM that is retrieved is signed by the correct
/// signing key.
const ACTOR_KEY_ANNOTATION: &str = "deislabs.io/actor-key";

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
    config: grpc::ContainerConfig,
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
    volumes: Vec<grpc::Mount>,
}

impl From<UserContainer> for grpc::Container {
    fn from(item: UserContainer) -> Self {
        grpc::Container {
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

impl From<UserContainer> for grpc::ContainerStats {
    fn from(item: UserContainer) -> Self {
        grpc::ContainerStats {
            attributes: Some(grpc::ContainerAttributes {
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

impl From<grpc::PodSandbox> for grpc::PodSandboxStatus {
    fn from(item: grpc::PodSandbox) -> Self {
        grpc::PodSandboxStatus {
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

#[derive(Clone, Debug, Default, PartialEq)]
struct UserSandbox {
    inner: grpc::PodSandbox,
    running_containers: Vec<String>,
}

/// Implement a CRI runtime service.
#[derive(Debug, Default)]
pub struct CriRuntimeService {
    module_store: Mutex<ModuleStore>,
    // NOTE: we could replace this with evmap or crossbeam
    sandboxes: RwLock<BTreeMap<String, UserSandbox>>,
    containers: RwLock<HashMap<String, UserContainer>>,
    running_containers: RwLock<HashMap<String, ContainerCancellationToken>>,
    pod_cidr: RwLock<Option<IpNet>>,
}

impl CriRuntimeService {
    pub async fn new(dir: PathBuf, pod_cidr: Option<IpNet>) -> Self {
        tokio::fs::create_dir_all(&dir)
            .await
            .expect("cannot create root directory for runtime service");
        CriRuntimeService {
            module_store: Mutex::new(ModuleStore::new(dir).await),
            sandboxes: RwLock::new(BTreeMap::default()),
            containers: RwLock::new(HashMap::new()),
            running_containers: RwLock::new(HashMap::new()),
            pod_cidr: RwLock::new(pod_cidr),
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
    async fn version(
        &self,
        req: Request<grpc::VersionRequest>,
    ) -> CriResult<grpc::VersionResponse> {
        log::info!("Version request from API version {:?}", req);
        Ok(Response::new(grpc::VersionResponse {
            version: API_VERSION.to_string(),
            runtime_name: env!("CARGO_PKG_NAME").to_string(),
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
            // NOTE: The Kubernetes API distinctly says that this MUST be a SemVer...
            // but actually require this format, which is not SemVer at all.
            runtime_api_version: RUNTIME_API_VERSION.to_string(),
        }))
    }

    async fn update_runtime_config(
        &self,
        req: Request<grpc::UpdateRuntimeConfigRequest>,
    ) -> CriResult<grpc::UpdateRuntimeConfigResponse> {
        let raw = req
            .into_inner()
            .runtime_config
            .unwrap_or_default()
            .network_config
            .unwrap_or_default()
            .pod_cidr;
        let pod_cidr = match raw.as_str() {
            "" => None,
            _ => Some(
                IpNet::from_str(&raw)
                    .map_err(|e| Status::invalid_argument(format!("invalid CIDR given: {}", e)))?,
            ),
        };

        let mut cidr = self.pod_cidr.write().await;
        *cidr = pod_cidr;
        Ok(Response::new(grpc::UpdateRuntimeConfigResponse {}))
    }

    async fn status(&self, req: Request<grpc::StatusRequest>) -> CriResult<grpc::StatusResponse> {
        let mut extra_info = HashMap::new();
        if req.into_inner().verbose {
            extra_info.insert(
                "running_sandboxes".to_owned(),
                self.sandboxes.read().await.len().to_string(),
            );
            extra_info.insert(
                "running_containers".to_owned(),
                self.containers.read().await.len().to_string(),
            );
        }

        Ok(Response::new(grpc::StatusResponse {
            status: Some(grpc::RuntimeStatus {
                conditions: vec![
                    // There isn't anything else to change on these right now,
                    // so keep them hard coded. If we start needing to update
                    // these (such as with networking) or add our own arbitrary
                    // conditions, we can move them into the struct
                    grpc::RuntimeCondition {
                        r#type: "RuntimeReady".to_owned(),
                        status: true,
                        // NOTE: We should make these reasons an enum once we
                        // actually define more of them
                        reason: "RuntimeStarted".to_owned(),
                        message: "Runtime has been started and is ready to run modules".to_owned(),
                    },
                    grpc::RuntimeCondition {
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
        req: Request<grpc::RunPodSandboxRequest>,
    ) -> CriResult<grpc::RunPodSandboxResponse> {
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

        let mut sandboxes = self.sandboxes.write().await;
        let id = Uuid::new_v4().to_string();
        sandboxes.insert(
            id.clone(),
            UserSandbox {
                inner: grpc::PodSandbox {
                    id: id.clone(),
                    metadata: sandbox_conf.metadata,
                    state: grpc::PodSandboxState::SandboxReady as i32,
                    created_at: Utc::now().timestamp_nanos(),
                    labels: sandbox_conf.labels,
                    annotations: sandbox_conf.annotations,
                    runtime_handler: handler.to_string(),
                },
                running_containers: vec![],
            },
        );

        Ok(Response::new(grpc::RunPodSandboxResponse {
            pod_sandbox_id: id,
        }))
    }

    async fn list_pod_sandbox(
        &self,
        req: Request<grpc::ListPodSandboxRequest>,
    ) -> CriResult<grpc::ListPodSandboxResponse> {
        let filter = req.into_inner().filter.unwrap_or_default();
        Ok(Response::new(grpc::ListPodSandboxResponse {
            items: self
                .sandboxes
                .read()
                .await
                .values()
                .map(|s| &s.inner)
                .filter(|sand| {
                    (filter.id == "" || sand.id == filter.id)
                        && filter
                            .state
                            .as_ref()
                            .map(|s| s.state == sand.state)
                            .unwrap_or(true)
                        && (filter.label_selector.is_empty()
                            || has_labels(&filter.label_selector, &sand.labels))
                })
                .cloned()
                .collect(),
        }))
    }

    async fn stop_pod_sandbox(
        &self,
        req: Request<grpc::StopPodSandboxRequest>,
    ) -> CriResult<grpc::StopPodSandboxResponse> {
        let id = req.into_inner().pod_sandbox_id;

        let mut sandboxes = self.sandboxes.write().await;
        let mut sandbox = match sandboxes.get_mut(&id) {
            Some(s) => s,
            None => return Err(Status::not_found(format!("Sandbox {} does not exist", id))),
        };

        // Stop all containers inside the sandbox. This forcibly terminates all containers with no grace period.
        for id in sandbox.running_containers.drain(0..) {
            self.stop_container(Request::new(grpc::StopContainerRequest {
                container_id: id.clone(),
                timeout: 0,
            }));
        }

        // mark the pod sandbox as not ready, preventing future container creation.
        sandbox.inner.state = grpc::PodSandboxState::SandboxNotready as i32;

        // TODO(bacongobbler): when networking is implemented, here is where we should tear down the network.

        Ok(Response::new(grpc::StopPodSandboxResponse {}))
    }

    // remove_pod_sandbox removes the sandbox. If there are running containers in the sandbox, they should be forcibly
    // removed.
    async fn remove_pod_sandbox(
        &self,
        req: Request<grpc::RemovePodSandboxRequest>,
    ) -> CriResult<grpc::RemovePodSandboxResponse> {
        let id = &req.into_inner().pod_sandbox_id;

        let mut sandboxes = self.sandboxes.write().await;
        let sandbox = match sandboxes.get(id) {
            Some(s) => s,
            None => return Err(Status::not_found(format!("Sandbox {} does not exist", id))),
        };

        // return an error if the sandbox container is still running.
        if sandbox.inner.state == grpc::PodSandboxState::SandboxReady as i32 {
            return Err(Status::failed_precondition(format!(
                "Sandbox container {} is not fully stopped",
                id
            )));
        }

        // TODO(bacongobbler): when networking is implemented, here is where we should return an error if the sandbox's
        // network namespace is not closed yet.

        // remove all containers inside the sandbox.
        for container_id in sandbox.running_containers.iter() {
            self.remove_container(Request::new(grpc::RemoveContainerRequest {
                container_id: container_id.clone(),
            }));
        }

        // remove the sandbox.
        sandboxes.remove(id);

        Ok(Response::new(grpc::RemovePodSandboxResponse {}))
    }

    async fn pod_sandbox_status(
        &self,
        req: Request<grpc::PodSandboxStatusRequest>,
    ) -> CriResult<grpc::PodSandboxStatusResponse> {
        let request = req.into_inner();

        let sandboxes = self.sandboxes.read().await;
        let sandbox = match sandboxes.get(&request.pod_sandbox_id) {
            Some(s) => s,
            None => {
                return Err(Status::not_found(format!(
                    "Sandbox {} does not exist",
                    request.pod_sandbox_id
                )))
            }
        };

        let status = grpc::PodSandboxStatus::from(sandbox.inner.clone());

        // TODO(bacongobbler): report back status on the network and linux-specific sandbox status here (when implemented)

        // TODO: generate any verbose information we want to report if requested.
        //
        // Right now, we don't have any extra information to provide to the user here, so we'll just return what we know.

        Ok(Response::new(grpc::PodSandboxStatusResponse {
            info: HashMap::new(),
            status: Some(status),
        }))
    }

    async fn create_container(
        &self,
        req: Request<grpc::CreateContainerRequest>,
    ) -> CriResult<grpc::CreateContainerResponse> {
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
            state: grpc::ContainerState::ContainerCreated as i32,
            created_at: Utc::now().timestamp_nanos(),
            config: container_config.to_owned(),
            log_path: None, // to be set further down
            image_ref: container_config.image.as_ref().unwrap().image.clone(), // FIXME(rylev): understand what it means for the image to be None
            volumes: vec![], // to be added further down
        };

        // create container root directory.
        let container_root_dir = self
            .module_store
            .lock()
            .await
            .root_dir()
            .join("containers")
            .join(&id);
        tokio::fs::create_dir_all(&container_root_dir).await?;

        // generate volume mounts.
        for mount in container_config.mounts {
            let volume_id = Uuid::new_v4().to_string();
            container.volumes.push(grpc::Mount {
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
            let log_path =
                PathBuf::from(&sandbox_config.log_directory).join(&container.config.log_path);
            tokio::fs::create_dir_all(&log_path).await?;
            container.log_path = Some(log_path);
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
        let mut sandboxes = self.sandboxes.write().await;
        sandboxes
            .get_mut(&container.pod_sandbox_id)
            .ok_or_else(|| {
                Status::not_found(format!(
                    "Could not found sandbox with id '{}'",
                    &container.pod_sandbox_id
                ))
            })?
            .running_containers
            .push(container.id.clone());
        self.containers
            .write()
            .await
            .insert(container.id.clone(), container);

        Ok(Response::new(grpc::CreateContainerResponse {
            container_id: id,
        }))
    }

    async fn start_container(
        &self,
        req: Request<grpc::StartContainerRequest>,
    ) -> CriResult<grpc::StartContainerResponse> {
        let id = req.into_inner().container_id;
        let mut containers = self.containers.write().await;

        // Create specific scope for the container read lock
        let mut container = containers
            .get_mut(&id)
            .ok_or_else(|| Status::not_found("Container not found"))?;
        let sandboxes = self.sandboxes.read().await;
        let sandbox = &sandboxes
            .get(&container.pod_sandbox_id)
            .ok_or_else(|| Status::not_found("Sandbox not found"))?
            .inner;

        let runtime = RuntimeHandler::from_string(&sandbox.runtime_handler)
            .map_err(|_| Status::invalid_argument("Invalid runtime handler"))?;

        let module_store = self.module_store.lock().await;

        // Get the WASM data from the image
        // TODO: handle error
        let image_ref =
            Reference::try_from(container.image_ref.clone()).expect("Failed to parse image_ref");
        let module_path = module_store
            .pull_file_path(&image_ref)
            .into_os_string()
            .into_string()
            .unwrap();

        let env: EnvVars = container
            .config
            .envs
            .iter()
            .cloned()
            .map(|pair| (pair.key, pair.value))
            .collect();

        match runtime {
            RuntimeHandler::WASCC => {
                // Load the WASM
                let wasm = tokio::fs::read(module_path).await?;
                // Get the key out of the request
                let key = container
                    .config
                    .annotations
                    .get(ACTOR_KEY_ANNOTATION)
                    .ok_or_else(|| Status::invalid_argument("actor key is required"))?;

                wascc_run_http(wasm, env, key).map_err(|e| Status::internal(e.to_string()))?;
                let mut running_containers = self.running_containers.write().await;

                // Fake token. Needs to be replaced with a real cancellation token, which should come from wascc.
                let token = ContainerCancellationToken::WasccCancelationToken(key.to_string());
                running_containers.insert(container.id.clone(), token);
            }
            RuntimeHandler::WASI => {
                let args = container.config.args.clone();
                let log_path = container.log_path.clone();
                let runtime = tokio::task::spawn_blocking(move || {
                    crate::wasm::WasiRuntime::new(
                        module_path,
                        env,
                        args,
                        // TODO: dirs
                        HashMap::new(),
                        log_path.as_ref(),
                    )
                })
                .await
                .expect("Failed to create new thread for creating runtime")
                .expect("Creating runtime failed");

                let token = RuntimeContainer::new(runtime).start();
                let mut running_containers = self.running_containers.write().await;
                running_containers.insert(container.id.clone(), token);
            }
        };
        container.state = grpc::ContainerState::ContainerRunning as i32;
        Ok(Response::new(grpc::StartContainerResponse {}))
    }

    async fn stop_container(
        &self,
        req: Request<grpc::StopContainerRequest>,
    ) -> CriResult<grpc::StopContainerResponse> {
        let tokens = self.running_containers.read().await;
        if let Some(token) = tokens.get(&req.into_inner().container_id) {
            token.stop()
        }
        Ok(Response::new(grpc::StopContainerResponse {}))
    }

    async fn remove_container(
        &self,
        req: Request<grpc::RemoveContainerRequest>,
    ) -> CriResult<grpc::RemoveContainerResponse> {
        let mut tokens = self.running_containers.write().await;
        let id = req.into_inner().container_id;
        match tokens.get(&id) {
            Some(token) => {
                token.remove();
                tokens.remove(&id);
            }
            None => {
                // Documentation seems to suggest that this is not an error case.
                log::debug!("ID {} is not found in running containers", id)
            }
        };

        let mut containers = self.containers.write().await;
        let container = containers.get_mut(&id).unwrap();

        let mut sandboxes = self.sandboxes.write().await;
        if let Some(sandbox) = sandboxes.get_mut(&container.pod_sandbox_id) {
            let pos = sandbox
                .running_containers
                .iter()
                .position(|id| &container.id == id)
                .unwrap();
            sandbox.running_containers.remove(pos);
        }
        //TODO(rylev): handle error of there not being a sandbox

        containers.remove(&id);

        Ok(Response::new(grpc::RemoveContainerResponse {}))
    }

    async fn list_containers(
        &self,
        req: Request<grpc::ListContainersRequest>,
    ) -> CriResult<grpc::ListContainersResponse> {
        let filter = req.into_inner().filter.unwrap_or_default();
        Ok(Response::new(grpc::ListContainersResponse {
            containers: self
                .containers
                .read()
                .await
                .values()
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
                .cloned()
                .map(grpc::Container::from)
                .collect(),
        }))
    }

    async fn container_status(
        &self,
        req: Request<grpc::ContainerStatusRequest>,
    ) -> CriResult<grpc::ContainerStatusResponse> {
        let id = req.into_inner().container_id;
        let containers = self.containers.read().await;
        let container = containers
            .get(&id)
            .ok_or_else(|| Status::not_found(format!("Container with ID {} does not exist", id)))?;

        Ok(Response::new(grpc::ContainerStatusResponse {
            status: Some(grpc::ContainerStatus {
                id: container.id.clone(),
                metadata: container.config.metadata.clone(),
                state: container.state,
                created_at: container.created_at,
                started_at: 0,
                finished_at: 0,
                exit_code: 0,
                image: container.config.image.clone(),
                image_ref: container.image_ref.clone(),
                reason: "because I said so".to_owned(),
                message: "hello earthlings".to_owned(),
                labels: container.config.labels.clone(),
                annotations: container.config.annotations.clone(),
                mounts: vec![],
                log_path: container
                    .log_path
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(""))
                    .into_os_string()
                    .into_string()
                    .unwrap(),
            }),
            info: HashMap::new(),
        }))
    }

    async fn container_stats(
        &self,
        req: Request<grpc::ContainerStatsRequest>,
    ) -> CriResult<grpc::ContainerStatsResponse> {
        let id = req.into_inner().container_id;
        let containers = self.containers.read().await;
        let container = containers
            .get(&id)
            .ok_or_else(|| Status::not_found(format!("Container with ID {} does not exist", id)))?;

        Ok(Response::new(grpc::ContainerStatsResponse {
            stats: Some(container.clone().into()),
        }))
    }

    async fn list_container_stats(
        &self,
        req: Request<grpc::ListContainerStatsRequest>,
    ) -> CriResult<grpc::ListContainerStatsResponse> {
        let filter = req.into_inner().filter.unwrap_or_default();
        let containers = self.containers.read().await;
        let container_stats: Vec<grpc::ContainerStats> = containers
            .values()
            .filter(|c| {
                (filter.id == "" || c.id == filter.id)
                    && (filter.pod_sandbox_id == "" || c.pod_sandbox_id == filter.pod_sandbox_id)
                    && (filter.label_selector.is_empty()
                        || has_labels(&filter.label_selector, &c.config.labels))
            })
            .cloned()
            .map(grpc::ContainerStats::from)
            .collect();
        Ok(Response::new(grpc::ListContainerStatsResponse {
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
    use ipnet::{IpNet, Ipv4Net};
    use std::net::Ipv4Addr;
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
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let res = svc
            .version(Request::new(grpc::VersionRequest::default()))
            .await;
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
    async fn test_update_runtime_config() {
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let _ = svc
            .update_runtime_config(Request::new(grpc::UpdateRuntimeConfigRequest {
                runtime_config: Some(grpc::RuntimeConfig {
                    network_config: Some(grpc::NetworkConfig {
                        pod_cidr: "192.168.1.0/24".to_owned(),
                    }),
                }),
            }))
            .await
            .expect("successful update config request");
        let set_cidr = svc.pod_cidr.read().await.unwrap();
        assert_eq!(
            IpNet::from(Ipv4Net::new(Ipv4Addr::new(192, 168, 1, 0), 24).unwrap()),
            set_cidr
        )
    }

    #[tokio::test]
    async fn test_status() {
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let res = svc
            .status(Request::new(grpc::StatusRequest::default()))
            .await
            .expect("successful status request");
        assert_eq!(
            res.get_ref().status.as_ref().unwrap().conditions,
            vec![
                // There isn't anything else to change on these right now,
                // so keep them hard coded. If we start needing to update
                // these (such as with networking) or add our own arbitrary
                // conditions, we can move them into the struct
                grpc::RuntimeCondition {
                    r#type: "RuntimeReady".to_owned(),
                    status: true,
                    // NOTE: We should make these reasons an enum once we
                    // actually define more of them
                    reason: "RuntimeStarted".to_owned(),
                    message: "Runtime has been started and is ready to run modules".to_owned(),
                },
                grpc::RuntimeCondition {
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
        let mut req = grpc::StatusRequest::default();
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
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let mut sandboxes = svc.sandboxes.write().await;
        let mut labels = HashMap::new();
        labels.insert("test1".to_owned(), "testing".to_owned());
        sandboxes.insert(
            "test".to_owned(),
            UserSandbox {
                inner: grpc::PodSandbox {
                    id: "test".to_owned(),
                    state: grpc::PodSandboxState::SandboxReady as i32,
                    labels: labels.clone(),
                    ..Default::default()
                },
                running_containers: vec![],
            },
        );
        sandboxes.insert(
            "test2".to_owned(),
            UserSandbox {
                inner: grpc::PodSandbox {
                    id: "test2".to_owned(),
                    state: grpc::PodSandboxState::SandboxReady as i32,
                    ..Default::default()
                },
                running_containers: vec![],
            },
        );
        sandboxes.insert(
            "test3".to_owned(),
            UserSandbox {
                inner: grpc::PodSandbox {
                    id: "test3".to_owned(),
                    state: grpc::PodSandboxState::SandboxNotready as i32,
                    ..Default::default()
                },
                running_containers: vec![],
            },
        );
        drop(sandboxes);
        let req = Request::new(grpc::ListPodSandboxRequest {
            filter: Some(grpc::PodSandboxFilter::default()),
        });
        let res = svc.list_pod_sandbox(req).await;
        // Nothing set should return all
        let sandboxes = res.expect("list sandboxes result").into_inner().items;
        assert_eq!(3, sandboxes.len());

        // Labels should only return matching containers
        let req = Request::new(grpc::ListPodSandboxRequest {
            filter: Some(grpc::PodSandboxFilter {
                label_selector: labels,
                ..Default::default()
            }),
        });
        let res = svc.list_pod_sandbox(req).await;
        let sandboxes = res.expect("list sandboxes result").into_inner().items;
        assert_eq!(1, sandboxes.len());

        // ID should return a specific sandbox
        let req = Request::new(grpc::ListPodSandboxRequest {
            filter: Some(grpc::PodSandboxFilter {
                id: "test2".to_owned(),
                ..Default::default()
            }),
        });
        let res = svc.list_pod_sandbox(req).await;
        let sandboxes = res.expect("list sandboxes result").into_inner().items;
        assert_eq!(1, sandboxes.len());

        // Status should return a specific sandbox
        let req = Request::new(grpc::ListPodSandboxRequest {
            filter: Some(grpc::PodSandboxFilter {
                state: Some(grpc::PodSandboxStateValue {
                    state: grpc::PodSandboxState::SandboxNotready as i32,
                }),
                ..Default::default()
            }),
        });
        let res = svc.list_pod_sandbox(req).await;
        let sandboxes = res.expect("list sandboxes result").into_inner().items;
        assert_eq!(1, sandboxes.len());
    }

    #[tokio::test]
    async fn test_pod_sandbox_status() {
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let mut sandboxes = svc.sandboxes.write().await;
        let sandbox = UserSandbox {
            inner: grpc::PodSandbox {
                id: "1".to_owned(),
                metadata: None,
                state: grpc::PodSandboxState::SandboxNotready as i32,
                created_at: Utc::now().timestamp_nanos(),
                labels: HashMap::new(),
                annotations: HashMap::new(),
                runtime_handler: RuntimeHandler::WASI.to_string(),
            },
            running_containers: vec![],
        };
        sandboxes.insert(sandbox.inner.id.clone(), sandbox);
        drop(sandboxes);
        let req = Request::new(grpc::PodSandboxStatusRequest {
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
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let mut container = UserContainer::default();
        container.id = "test".to_owned();
        container.pod_sandbox_id = "1".to_owned();
        let container_id = container.id.clone();
        svc.containers
            .write()
            .await
            .insert(container_id.clone(), container);

        let mut sandboxes = svc.sandboxes.write().await;
        sandboxes.insert(
            "1".to_owned(),
            UserSandbox {
                inner: grpc::PodSandbox {
                    id: "1".to_owned(),
                    metadata: None,
                    state: grpc::PodSandboxState::SandboxNotready as i32,
                    created_at: Utc::now().timestamp_nanos(),
                    labels: HashMap::new(),
                    annotations: HashMap::new(),
                    runtime_handler: RuntimeHandler::WASI.to_string(),
                },
                running_containers: vec![container_id],
            },
        );
        drop(sandboxes);
        let req = Request::new(grpc::RemovePodSandboxRequest {
            pod_sandbox_id: "1".to_owned(),
        });
        let res = svc.remove_pod_sandbox(req).await;
        // we expect an empty response object
        res.expect("remove sandbox result");
        // TODO(bacongobbler): un-comment this once remove_container() has been implemented
        //assert_eq!(0, svc.containers.read().unwrap().len());

        assert_eq!(0, svc.sandboxes.read().await.values().len());
    }

    #[tokio::test]
    async fn test_stop_pod_sandbox() {
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let mut sandboxes = svc.sandboxes.write().await;
        sandboxes.insert(
            "test".to_owned(),
            UserSandbox {
                inner: grpc::PodSandbox {
                    id: "test".to_owned(),
                    metadata: None,
                    state: grpc::PodSandboxState::SandboxReady as i32,
                    created_at: Utc::now().timestamp_nanos(),
                    labels: HashMap::new(),
                    annotations: HashMap::new(),
                    runtime_handler: RuntimeHandler::WASI.to_string(),
                },
                running_containers: vec![],
            },
        );
        drop(sandboxes);
        let req = Request::new(grpc::StopPodSandboxRequest {
            pod_sandbox_id: "test".to_owned(),
        });
        let res = svc.stop_pod_sandbox(req).await;

        // Expect the stopped ID to be the same as the requested ID.
        res.expect("empty stop result");

        // test what happens when the requested pod sandbox doesn't exist
        let req = Request::new(grpc::StopPodSandboxRequest {
            pod_sandbox_id: "doesnotexist".to_owned(),
        });
        let res = svc.stop_pod_sandbox(req).await;
        res.expect_err("pod sandbox does not exist");
    }

    #[tokio::test]
    async fn test_create_container() {
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let mut sandboxes = svc.sandboxes.write().await;
        sandboxes.insert(
            "test".to_owned(),
            UserSandbox {
                inner: grpc::PodSandbox {
                    id: "test".to_owned(),
                    metadata: None,
                    state: grpc::PodSandboxState::SandboxReady as i32,
                    created_at: Utc::now().timestamp_nanos(),
                    labels: HashMap::new(),
                    annotations: HashMap::new(),
                    runtime_handler: RuntimeHandler::WASI.to_string(),
                },
                running_containers: vec![],
            },
        );
        drop(sandboxes);
        let mut config = grpc::ContainerConfig::default();
        config.image = Some(grpc::ImageSpec {
            image: "foo/bar:baz".to_owned(),
        });
        let req = Request::new(grpc::CreateContainerRequest {
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
        assert_eq!(1, svc.containers.read().await.len());
    }

    #[tokio::test]
    async fn test_start_container() {
        // Put every file in a temp dir so it's automatically cleaned up
        let dir = tempdir().expect("Couldn't create temp directory");
        let svc = CriRuntimeService::new(dir.path().to_owned(), None).await;

        let image_ref = Reference::try_from("foo/bar:baz".to_owned()).unwrap();

        let module_store = ModuleStore::new(dir.path().to_path_buf()).await;

        // create temp directories
        let log_dir_name = dir.path().join("testdir");
        let image_file = module_store.pull_file_path(&image_ref);
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
        let sandbox = grpc::PodSandbox::default();
        let mut container = UserContainer::default();

        let container_id = {
            // write container
            let mut containers = svc.containers.write().await;
            container.pod_sandbox_id = sandbox.id.clone();
            container.log_path = Some(log_dir_name);
            container.image_ref = image_ref.into();
            let container_id = container.id.clone();

            containers.insert(container.id.clone(), container);

            // write sandbox
            let mut sandboxes = svc.sandboxes.write().await;
            sandboxes.insert(
                sandbox.id.clone(),
                UserSandbox {
                    inner: sandbox,
                    running_containers: vec![],
                },
            );
            container_id
        };

        let id = container_id.clone();
        let req = Request::new(grpc::StartContainerRequest { container_id });
        let res = svc.start_container(req).await;
        // We expect an empty response object
        res.expect("start container result");
        // We should also expect the container to be in the running state
        let containers = svc.containers.read().await;
        assert_eq!(
            grpc::ContainerState::ContainerRunning as i32,
            containers[&id].state
        );
    }

    #[tokio::test]
    async fn test_stop_container() {
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let req = Request::new(grpc::StopContainerRequest::default());
        let res = svc.stop_container(req).await;
        // We expect an empty response object
        res.expect("stop container result");
    }

    #[tokio::test]
    async fn test_remove_container() {
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let mut containers = svc.containers.write().await;

        containers.insert(
            "test".to_owned(),
            UserContainer {
                id: "test".to_owned(),
                pod_sandbox_id: "test".to_owned(),
                image_ref: "doesntmatter".to_owned(),
                created_at: Utc::now().timestamp_nanos(),
                state: grpc::ContainerState::ContainerRunning as i32,
                config: grpc::ContainerConfig::default(),
                log_path: None,
                volumes: Vec::default(),
            },
        );
        containers.insert(
            "foo".to_owned(),
            UserContainer {
                id: "foo".to_owned(),
                pod_sandbox_id: "foo".to_owned(),
                image_ref: "doesntmatter".to_owned(),
                created_at: Utc::now().timestamp_nanos(),
                state: grpc::ContainerState::ContainerRunning as i32,
                config: grpc::ContainerConfig::default(),
                log_path: None,
                volumes: Vec::default(),
            },
        );
        drop(containers);
        let req = Request::new(grpc::RemoveContainerRequest {
            container_id: "test".to_owned(),
        });
        let res = svc.remove_container(req).await;
        // We expect an empty response object
        res.expect("remove container result");
        // Check for the container to be gone and that we still have one left
        assert_eq!(1, svc.containers.read().await.len());
    }

    #[tokio::test]
    async fn test_list_containers() {
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let mut containers = svc.containers.write().await;
        let mut labels = HashMap::new();
        labels.insert("test1".to_owned(), "testing".to_owned());
        containers.insert(
            "test".to_owned(),
            UserContainer {
                id: "test".to_owned(),
                pod_sandbox_id: "test".to_owned(),
                state: grpc::ContainerState::ContainerRunning as i32,
                config: grpc::ContainerConfig {
                    metadata: Some(grpc::ContainerMetadata {
                        attempt: 1,
                        name: "test".to_owned(),
                    }),
                    labels: labels.clone(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        containers.insert(
            "test2".to_owned(),
            UserContainer {
                id: "test2".to_owned(),
                pod_sandbox_id: "test2".to_owned(),
                state: grpc::ContainerState::ContainerRunning as i32,
                config: grpc::ContainerConfig {
                    metadata: Some(grpc::ContainerMetadata {
                        attempt: 1,
                        name: "test2".to_owned(),
                    }),
                    labels: HashMap::default(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        containers.insert(
            "test3".to_owned(),
            UserContainer {
                id: "test3".to_owned(),
                pod_sandbox_id: "test2".to_owned(),
                state: grpc::ContainerState::ContainerCreated as i32,
                config: grpc::ContainerConfig {
                    metadata: Some(grpc::ContainerMetadata {
                        attempt: 1,
                        name: "test2".to_owned(),
                    }),
                    labels: HashMap::default(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        drop(containers);
        let req = Request::new(grpc::ListContainersRequest {
            filter: Some(grpc::ContainerFilter::default()),
        });
        let res = svc.list_containers(req).await;
        // Nothing set should return all
        let containers = res.expect("list containers result").into_inner().containers;
        assert_eq!(3, containers.len());

        // Pod sandbox ID should return all containers in the given sandbox
        let req = Request::new(grpc::ListContainersRequest {
            filter: Some(grpc::ContainerFilter {
                pod_sandbox_id: "test2".to_owned(),
                ..Default::default()
            }),
        });
        let res = svc.list_containers(req).await;
        let containers = res.expect("list containers result").into_inner().containers;
        assert_eq!(2, containers.len());

        // Labels should only return matching containers
        let req = Request::new(grpc::ListContainersRequest {
            filter: Some(grpc::ContainerFilter {
                label_selector: labels,
                ..Default::default()
            }),
        });
        let res = svc.list_containers(req).await;
        let containers = res.expect("list containers result").into_inner().containers;
        assert_eq!(1, containers.len());

        // ID and sandbox ID should return a specific container
        let req = Request::new(grpc::ListContainersRequest {
            filter: Some(grpc::ContainerFilter {
                id: "test2".to_owned(),
                pod_sandbox_id: "test2".to_owned(),
                ..Default::default()
            }),
        });
        let res = svc.list_containers(req).await;
        let containers = res.expect("list containers result").into_inner().containers;
        assert_eq!(1, containers.len());

        // Status should return a specific container
        let req = Request::new(grpc::ListContainersRequest {
            filter: Some(grpc::ContainerFilter {
                state: Some(grpc::ContainerStateValue {
                    state: grpc::ContainerState::ContainerCreated as i32,
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
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let mut containers = svc.containers.write().await;
        containers.insert(
            "test".to_owned(),
            UserContainer {
                id: "test".to_owned(),
                pod_sandbox_id: "test".to_owned(),
                config: grpc::ContainerConfig {
                    metadata: Some(grpc::ContainerMetadata {
                        attempt: 1,
                        name: "test".to_owned(),
                    }),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        drop(containers);
        let req = Request::new(grpc::ContainerStatusRequest {
            container_id: "test".to_owned(),
            verbose: false,
        });
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
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let mut containers = svc.containers.write().await;
        let mut labels = HashMap::new();
        labels.insert("test1".to_owned(), "testing".to_owned());
        containers.insert(
            "test".to_owned(),
            UserContainer {
                id: "test".to_owned(),
                pod_sandbox_id: "test".to_owned(),
                config: grpc::ContainerConfig {
                    metadata: Some(grpc::ContainerMetadata {
                        attempt: 1,
                        name: "test".to_owned(),
                    }),
                    labels: labels.clone(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        drop(containers);
        let req = Request::new(grpc::ContainerStatsRequest {
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
            grpc::ContainerStats {
                attributes: Some(grpc::ContainerAttributes {
                    id: "test".to_owned(),
                    metadata: Some(grpc::ContainerMetadata {
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
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let mut containers = svc.containers.write().await;
        let mut labels = HashMap::new();
        labels.insert("test1".to_owned(), "testing".to_owned());
        containers.insert(
            "test".to_owned(),
            UserContainer {
                id: "test".to_owned(),
                pod_sandbox_id: "test".to_owned(),
                config: grpc::ContainerConfig {
                    metadata: Some(grpc::ContainerMetadata {
                        attempt: 1,
                        name: "test".to_owned(),
                    }),
                    labels: labels.clone(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        containers.insert(
            "test2".to_owned(),
            UserContainer {
                id: "test2".to_owned(),
                pod_sandbox_id: "test2".to_owned(),
                config: grpc::ContainerConfig {
                    metadata: Some(grpc::ContainerMetadata {
                        attempt: 1,
                        name: "test2".to_owned(),
                    }),
                    labels: HashMap::default(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        containers.insert(
            "test3".to_owned(),
            UserContainer {
                id: "test3".to_owned(),
                pod_sandbox_id: "test2".to_owned(),
                config: grpc::ContainerConfig {
                    metadata: Some(grpc::ContainerMetadata {
                        attempt: 1,
                        name: "test2".to_owned(),
                    }),
                    labels: HashMap::default(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        drop(containers);
        let req = Request::new(grpc::ListContainerStatsRequest {
            filter: Some(grpc::ContainerStatsFilter::default()),
        });
        let res = svc.list_container_stats(req).await;
        // Nothing set should return all
        let stats = res.expect("list container stats result").into_inner().stats;
        assert_eq!(3, stats.len());

        // Pod sandbox ID should return all containers in the given sandbox
        let req = Request::new(grpc::ListContainerStatsRequest {
            filter: Some(grpc::ContainerStatsFilter {
                pod_sandbox_id: "test2".to_owned(),
                ..Default::default()
            }),
        });
        let res = svc.list_container_stats(req).await;
        let stats = res.expect("list container stats result").into_inner().stats;
        assert_eq!(2, stats.len());

        // Labels should only return matching containers
        let req = Request::new(grpc::ListContainerStatsRequest {
            filter: Some(grpc::ContainerStatsFilter {
                label_selector: labels,
                ..Default::default()
            }),
        });
        let res = svc.list_container_stats(req).await;
        let stats = res.expect("list container stats result").into_inner().stats;
        assert_eq!(1, stats.len());

        // ID and sandbox ID should return a specific container
        let req = Request::new(grpc::ListContainerStatsRequest {
            filter: Some(grpc::ContainerStatsFilter {
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
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let mut sandbox_req = grpc::RunPodSandboxRequest::default();
        sandbox_req.runtime_handler = RuntimeHandler::WASI.to_string();

        // Create a temporary log dir for testing purposes
        let dir = tempdir().unwrap();
        let log_dir_name = dir.path().join("testdir");
        let mut conf = grpc::PodSandboxConfig::default();
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
        let svc = CriRuntimeService::new(PathBuf::from(""), None).await;
        let mut sandbox_req = grpc::RunPodSandboxRequest::default();
        sandbox_req.runtime_handler = RuntimeHandler::WASI.to_string();

        // Create a temporary log dir for testing purposes
        let dir = tempdir().unwrap();
        let log_dir_name = dir.path().join("testdir");
        let mut conf = grpc::PodSandboxConfig::default();
        conf.log_directory = log_dir_name.to_str().unwrap().to_owned();
        sandbox_req.config = Some(conf);
        let req = Request::new(sandbox_req);
        let res = svc.run_pod_sandbox(req).await;

        let id = res.unwrap().get_ref().pod_sandbox_id.clone();
        let list_req = Request::new(grpc::ListPodSandboxRequest::default());
        let res = svc.list_pod_sandbox(list_req).await;
        let sandboxes = res.expect("successful pod list").get_ref().items.clone();
        assert_eq!(1, sandboxes.len());
        // And make sure the UID returned actually exists
        assert_eq!(id, sandboxes[0].id);
    }
}

pub struct RuntimeContainer {
    handle: JoinHandle<Result<()>>,
    sender: UnboundedSender<()>,
}

impl RuntimeContainer {
    pub fn new<T: Runtime + Send + 'static>(rt: T) -> Self {
        let (sender, mut receiver) = unbounded_channel::<()>();
        let handle = tokio::spawn(async move {
            receiver.recv().await.unwrap();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = rt.run() {
                    // TODO(taylor): Implement messaging here to indicate that there was a problem running the module
                    error!("Error while running module: {}", e);
                }
                Ok(())
            })
            .await
            .unwrap()
        });
        RuntimeContainer { handle, sender }
    }

    pub fn start(self) -> ContainerCancellationToken {
        self.sender.send(()).unwrap();
        ContainerCancellationToken::WasiCancelationToken(self.handle)
    }
}

type WasccPublicKey = String;

#[derive(Debug)]
pub enum ContainerCancellationToken {
    WasccCancelationToken(WasccPublicKey),
    WasiCancelationToken(JoinHandle<Result<()>>),
}

impl ContainerCancellationToken {
    fn stop(&self) {
        match self {
            Self::WasccCancelationToken(key) => {
                //Remove container
                if let Err(e) = wascc_stop(key) {
                    info!("wascc module was not stopped: {}", e.to_string());
                }
            }
            Self::WasiCancelationToken(_handle) => {
                todo!("Stopping a running container is not currently supported");
            }
        }
    }
    fn remove(&self) {
        match self {
            Self::WasccCancelationToken(key) => {
                //Remove container
                if let Err(e) = wascc_stop(key) {
                    info!("wascc module was not stopped: {}", e.to_string());
                }
            }
            Self::WasiCancelationToken(_handle) => {
                todo!("Removing a running container is not currently supported");
            }
        }
    }
}
