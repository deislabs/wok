use std::collections::HashMap;

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use log::info;
use wascc_host::{host, Actor, NativeCapability};

/// The name of the HTTP capability.
const HTTP_CAPABILITY: &str = "wascc:http_server";

/// Kubernetes' view of environment variables is an unordered map of string to string.
pub type EnvVars = std::collections::HashMap<String, String>;

#[cfg(target_os = "linux")]
const HTTP_LIB: &str = "./lib/libwascc_httpsrv.so";
#[cfg(target_os = "macos")]
const HTTP_LIB: &str = "./lib/libwascc_httpsrv.dylib";

struct WasccState {
    actor_id: String,
    waker: Option<Waker>,
    completed: bool,
}

struct WasccFuture {
    state: Arc<Mutex<WasccState>>,
}
impl WasccFuture {
    pub fn new(data: Vec<u8>, key: &str, capabilities: Vec<Capability>) -> Self {
        // TODO: Need to handle this error better.
        // We could either delay start until poll() is called, or we could
        // capture the error and report it.
        wascc_run(data, key, capabilities).expect("start WaSCC run");
        let state = Arc::new(Mutex::new(WasccState {
            actor_id: key.to_string(),
            waker: None,
            completed: false,
        }));

        // All we have to do here is wake the waker when the actor disappears from the list.
        let inner_state = state.clone();
        tokio::spawn(async move {
            let mut run = inner_state.lock().unwrap();
            // As long as this is true, the actor is running.
            // TODO: Figure out a good way to allow a yield/sleep here.
            while host::actors().iter().any(|(id, _)| *id == run.actor_id) {}
            run.completed = true;
            if let Some(waker) = run.waker.take() {
                waker.wake();
            }
        });

        WasccFuture { state }
    }
}

impl Future for WasccFuture {
    type Output = Result<(), failure::Error>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.state.lock().unwrap();
        if state.completed {
            // I cannot figure out how to get a runtime failure out of waSCC.
            Poll::Ready(Ok(()))
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// This registers all of the native capabilities known to this host.
///
/// In the future, we'll do this dynamically. For now, though, these are the
/// caps that we know we need in order to wire up Kubernetes
pub fn register_native_capabilities() -> Result<(), failure::Error> {
    let data = NativeCapability::from_file(HTTP_LIB)
        .map_err(|e| format_err!("Failed to read HTTP capability {}: {}", HTTP_LIB, e))?;
    host::add_native_capability(data)
        .map_err(|e| format_err!("Failed to load HTTP capability: {}", e))
}

/// Run a WasCC module inside of the host, configuring it to handle HTTP requests.
///
/// This bootstraps an HTTP host, using the value of the env's `PORT` key to expose a port.
pub fn wascc_run_http(data: Vec<u8>, env: EnvVars, key: &str) -> Result<(), failure::Error> {
    let mut httpenv: HashMap<String, String> = HashMap::new();
    httpenv.insert(
        "PORT".into(),
        env.get("PORT")
            .map(|a| a.to_string())
            .unwrap_or_else(|| "80".to_string()),
    );

    wascc_run(
        data,
        key,
        vec![Capability {
            name: HTTP_CAPABILITY,
            env,
        }],
    )
}

/// Stop a running waSCC actor.
pub fn wascc_stop(key: &str) -> Result<(), wascc_host::errors::Error> {
    host::remove_actor(key)
}

/// Capability describes a waSCC capability.
///
/// Capabilities are made available to actors through a two-part processthread:
/// - They must be registered
/// - For each actor, the capability must be configured
pub struct Capability {
    name: &'static str,
    env: EnvVars,
}

/// Run the given WASM data as a waSCC actor with the given public key.
///
/// The provided capabilities will be configured for this actor, but the capabilities
/// must first be loaded into the host by some other process, such as register_native_capabilities().
pub fn wascc_run(
    data: Vec<u8>,
    key: &str,
    capabilities: Vec<Capability>,
) -> Result<(), failure::Error> {
    info!("wascc run");
    let load = Actor::from_bytes(data).map_err(|e| format_err!("Error loading WASM: {}", e))?;
    host::add_actor(load).map_err(|e| format_err!("Error adding actor: {}", e))?;

    capabilities.iter().try_for_each(|cap| {
        info!("configuring capability {}", cap.name);
        host::configure(key, cap.name, cap.env.clone())
            .map_err(|e| format_err!("Error configuring capabilities for module: {}", e))
    })?;
    info!("Instance executing");
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    const GREET_ACTOR_KEY: &str = "MADK3R3H47FGXN5F4HWPSJH4WCKDWKXQBBIOVI7YEPEYEMGJ2GDFIFE5";
    const ECHO_ACTOR_KEY: &str = "MDAYLDTOZEHQFPB3CL5PAFY5UTNCW32P54XGWYX3FOM2UBRYNCP3I3BF";

    #[cfg(target_os = "linux")]
    const ECHO_LIB: &str = "./lib/libecho_provider.so";
    #[cfg(target_os = "macos")]
    const ECHO_LIB: &str = "./lib/libecho_provider.dylib";

    #[test]
    fn test_register_native_capabilities() {
        register_native_capabilities().expect("HTTP capability is registered");
    }

    #[test]
    fn test_wascc_echo() {
        let data = NativeCapability::from_file(ECHO_LIB).expect("loaded echo library");
        host::add_native_capability(data).expect("added echo capability");

        let wasm = std::fs::read("./testdata/echo_actor_signed.wasm").expect("load echo WASM");
        // TODO: use wascc_run to execute echo_actor
        wascc_run(
            wasm,
            ECHO_ACTOR_KEY,
            vec![Capability {
                name: "wok:echoProvider",
                env: EnvVars::new(),
            }],
        )
        .expect("completed echo run")
    }

    #[tokio::test]
    async fn test_wascc_future() {
        let wasm = std::fs::read("./testdata/greet_actor_signed.wasm").expect("read the wasm file");

        let runner = tokio::spawn(async {
            let mut httpenv: HashMap<String, String> = HashMap::new();
            httpenv.insert("PORT".into(), "8707".into());

            WasccFuture::new(
                wasm,
                GREET_ACTOR_KEY,
                vec![Capability {
                    name: HTTP_CAPABILITY,
                    env: httpenv,
                }],
            )
            .await
            .expect("completed wascc run");
        });

        // This waits 5 seconds and then stops the actor, which should end the await below.
        tokio::spawn(async move {
            std::thread::sleep(std::time::Duration::from_secs(5));
            wascc_stop(GREET_ACTOR_KEY).expect("stopped wascc echo actor");
        })
        .await
        .expect("waiter waited long enough");

        runner.await.expect("wascc actor was removed");
    }

    /* Currently, an actor can only be run ONCE per host, which means we can't run this
     * test and the future test. We need a fix/workaround for this.
    #[test]
    fn test_wascc_run() {
        //register_native_capabilities().expect("HTTP capability is registered");
        // Open file
        let data = std::fs::read("./testdata/greet_actor_signed.wasm").expect("read the wasm file");
        // Send into wascc_run
        wascc_run_http(data, EnvVars::new(), GREET_ACTOR_KEY)
            .expect("successfully executed a WASM");

        // Give the webserver a chance to start up.
        std::thread::sleep(std::time::Duration::from_secs(13));
        wascc_stop(GREET_ACTOR_KEY).expect("Removed the actor");
    }
    */
}
