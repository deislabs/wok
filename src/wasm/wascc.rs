use log::info;
use std::collections::HashMap;
use wascc_host::{host, Actor, NativeCapability};

const HTTP_CAPABILITY: &str = "wascc:http_server";

/// Kubernetes' view of environment variables is an unordered map of string to string.
type EnvVars = std::collections::HashMap<String, String>;

/// This registers all of the native capabilities known to this host.
///
/// In the future, we'll do this dynamically. For now, though, these are the
/// caps that we know we need in order to wire up Kubernetes
pub fn register_native_capabilities() -> Result<(), failure::Error> {
    let httplib = "./lib/libwascc_httpsrv.dylib";
    // The match is to unwrap an error from a thread and convert it to a type that
    // can cross the thread boundary. There is surely a better way.
    let data = NativeCapability::from_file(httplib)
        .map_err(|e| format_err!("Failed to read HTTP capability {}: {}", httplib, e))?;
    host::add_native_capability(data)
        .map(|_| ())
        .map_err(|e| format_err!("Failed to load HTTP capability: {}", e))
}

/// Run a WasCC module inside of the host.
pub fn wascc_run(data: &[u8], env: EnvVars, key: &str) -> Result<(), failure::Error> {
    let load =
        Actor::from_bytes(data.to_vec()).map_err(|e| format_err!("Error loading WASM: {}", e))?;
    host::add_actor(load).map_err(|e| format_err!("Error adding actor: {}", e))?;

    let mut httpenv: HashMap<String, String> = HashMap::new();
    httpenv.insert(
        "PORT".into(),
        env.get("PORT")
            .map(|a| a.to_string())
            .unwrap_or_else(|| "80".to_string()),
    );

    // TODO: Middleware provider for env vars
    let _ = host::configure(key, HTTP_CAPABILITY, httpenv)
        .map_err(|e| format_err!("Error configuring HTTP server for module: {}", e))?;
    info!("Instance executing");
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_register_native_capabilities() {
        register_native_capabilities().expect("HTTP capability is registered");
    }

    #[test]
    fn test_wascc_run() {
        register_native_capabilities().expect("HTTP capability is registered");
        // Open file
        let data = std::fs::read("./lib/greet_actor_signed.wasm").expect("read the wasm file");
        // Send into wascc_run
        wascc_run(
            &data,
            EnvVars::new(),
            "MADK3R3H47FGXN5F4HWPSJH4WCKDWKXQBBIOVI7YEPEYEMGJ2GDFIFE5",
        )
        .expect("successfully executed a WASM");

        host::remove_actor("MADK3R3H47FGXN5F4HWPSJH4WCKDWKXQBBIOVI7YEPEYEMGJ2GDFIFE5")
            .expect("Removed the actor");
    }
}
