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
    match NativeCapability::from_file(httplib) {
        Err(e) => Err(format_err!(
            "Failed to read HTTP capability {}: {}",
            httplib,
            e
        )),
        Ok(data) => match host::add_native_capability(data) {
            Err(e) => Err(format_err!("Failed to load HTTP capability: {}", e)),
            Ok(_) => Ok(()),
        },
    }
}

/// Run a WasCC module inside of the host.
pub fn wascc_run(data: &[u8], env: EnvVars, key: &str) -> Result<(), failure::Error> {
    let load = match Actor::from_bytes(data.to_vec()) {
        Err(e) => return Err(format_err!("Error loading WASM: {}", e.to_string())),
        Ok(data) => data,
    };
    if let Err(e) = host::add_actor(load) {
        return Err(format_err!("Error adding actor: {}", e.to_string()));
    }
    let mut httpenv: HashMap<String, String> = HashMap::new();
    httpenv.insert(
        "PORT".into(),
        env.get("PORT")
            .map(|a| a.to_string())
            .unwrap_or_else(|| "80".to_string()),
    );
    // TODO: Middleware provider for env vars
    match host::configure(key, HTTP_CAPABILITY, httpenv) {
        Err(e) => {
            return Err(format_err!(
                "Error configuring HTTP server for module: {}",
                e.to_string()
            ));
        }
        Ok(_) => {
            info!("Instance executing");
        }
    }
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
        // Read bytes
        // Send into wascc_run
    }
}
