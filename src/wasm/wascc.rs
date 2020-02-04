use log::info;
use std::collections::HashMap;
use wascc_host::{host, Actor, NativeCapability};

const HTTP_CAPABILITY: &str = "wascc:http_server";

/// Kubernetes' view of environment variables is an unordered map of string to string.
type EnvVars = std::collections::HashMap<String, String>;

#[cfg(target_os = "linux")]
const HTTP_LIB: &str = "./lib/libwascc_httpsrv.so";
#[cfg(target_os = "macos")]
const HTTP_LIB: &str = "./lib/libwascc_httpsrv.dylib";

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

/// Run a WasCC module inside of the host.
pub fn wascc_run_http(data: &[u8], env: EnvVars, key: &str) -> Result<(), failure::Error> {
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
            name: HTTP_CAPABILITY.to_owned(),
            env,
        }],
    )
}

pub struct Capability {
    name: String,
    env: EnvVars,
}

pub fn wascc_run(
    data: &[u8],
    key: &str,
    capabilities: Vec<Capability>,
) -> Result<(), failure::Error> {
    let load =
        Actor::from_bytes(data.to_vec()).map_err(|e| format_err!("Error loading WASM: {}", e))?;
    host::add_actor(load).map_err(|e| format_err!("Error adding actor: {}", e))?;

    capabilities.iter().try_for_each(|cap| {
        host::configure(key, cap.name.as_str(), cap.env.clone())
            .map_err(|e| format_err!("Error configuring HTTP server for module: {}", e))
    })?;
    info!("Instance executing");
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[cfg(target_os = "linux")]
    const ECHO_LIB: &str = "../lib/libecho_provider.so";
    #[cfg(target_os = "macos")]
    const ECHO_LIB: &str = "./lib/libecho_provider.dylib";

    #[test]
    fn test_register_native_capabilities() {
        register_native_capabilities().expect("HTTP capability is registered");
    }

    #[test]
    fn test_wascc_run() {
        //register_native_capabilities().expect("HTTP capability is registered");
        // Open file
        let data = std::fs::read("./lib/greet_actor_signed.wasm").expect("read the wasm file");
        // Send into wascc_run
        wascc_run_http(
            &data,
            EnvVars::new(),
            "MADK3R3H47FGXN5F4HWPSJH4WCKDWKXQBBIOVI7YEPEYEMGJ2GDFIFE5",
        )
        .expect("successfully executed a WASM");

        host::remove_actor("MADK3R3H47FGXN5F4HWPSJH4WCKDWKXQBBIOVI7YEPEYEMGJ2GDFIFE5")
            .expect("Removed the actor");
    }

    #[test]
    fn test_wascc_echo() {
        let data = NativeCapability::from_file(ECHO_LIB).expect("loaded echo library");
        host::add_native_capability(data).expect("added echo capability");

        // TODO: use wascc_run to execute echo_actor
    }
}
