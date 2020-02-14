use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use super::{Result, Runtime};
use log::info;
use tempfile::NamedTempFile;
use wasi_common::*;
use wasmtime::*;
use wasmtime_wasi::*;

/// WasiRuntime provides a WASI compatible runtime. A runtime should be used for
/// each "instance" of a process and can be passed to a thread pool for running
pub struct WasiRuntime {
    /// binary module data to be run as a wasm module
    module_data: Vec<u8>,
    /// key/value environment variables made available to the wasm process
    env: HashMap<String, String>,
    /// the arguments passed as the command-line arguments list
    args: Vec<String>,
    /// a hash map of local file system paths to optional path names in the runtime
    /// (e.g. /tmp/foo/myfile -> /app/config). If the optional value is not given,
    /// the same path will be allowed in the runtime
    dirs: HashMap<String, Option<String>>,
    /// Handle to stdout
    stdout: Option<NamedTempFile>,
    /// handle to stderr
    stderr: Option<NamedTempFile>,
}

impl Runtime for WasiRuntime {
    fn run(&self) -> Result<()> {
        let engine = HostRef::new(Engine::default());
        let store = Store::new(&engine);

        // Build the WASI instance and then generate a list of WASI modules
        let global_exports = store.global_exports().clone();
        let store = HostRef::new(store);

        let ctx_builder = WasiCtxBuilder::new().args(&self.args).envs(&self.env);
        let ctx_builder = match &self.stdout {
            Some(f) => ctx_builder.stdout(f.reopen()?),
            None => ctx_builder,
        };

        let mut ctx_builder = match &self.stderr {
            Some(f) => ctx_builder.stderr(f.reopen()?),
            None => ctx_builder,
        };

        for (key, value) in self.dirs.iter() {
            let guest_dir = value.as_ref().unwrap_or(key);
            // Try and preopen the directory and then try to clone it. This step adds the directory to the context
            ctx_builder = ctx_builder.preopened_dir(preopen_dir(key)?, guest_dir);
        }
        let wasi_ctx = ctx_builder.build()?;
        let wasi_inst = wasmtime::Instance::from_handle(
            &store,
            instantiate_wasi_with_context(global_exports, wasi_ctx)?,
        );
        let module = Module::new(&store, &self.module_data)
            .map_err(|e| format_err!("unable to load module data {}", e))?;
        let module = HostRef::new(module);
        // Iterate through the module includes and resolve imports
        let imports = module
            .borrow()
            .imports()
            .iter()
            .map(|i| {
                let module_name = i.module().as_str();
                let field_name = i.name().as_str();
                if let Some(export) = wasi_inst.find_export_by_name(field_name) {
                    Ok(export.clone())
                } else {
                    failure::bail!(
                        "Import {} was not found in module {}",
                        field_name,
                        module_name
                    )
                }
            })
            .collect::<Result<Vec<_>>>()?;

        info!("starting run of module");
        let _instance = Instance::new(&store, &module, &imports)
            .map_err(|e| format_err!("unable to run module: {}", e))?;

        info!("module run complete");
        Ok(())
    }

    /// output returns a tuple of BufReaders containing stdout and stderr
    /// respectively. It will error if it can't open a stream
    // TODO(taylor): I can't completely tell from documentation, but we may
    // need to switch this out from a BufReader if it can't handle streaming
    // logs
    fn output(&self) -> Result<(BufReader<File>, BufReader<File>)> {
        // As warned in the BufReader docs, creating multiple BufReaders on the
        // same stream can cause data loss. So reopen a new file object each
        // time this function as called so as to not drop any data
        let stdout = match &self.stdout {
            Some(s) => s,
            None => return Err(format_err!("logging is not enabled for this runtime")),
        }
        .reopen()?;
        let stderr = match &self.stderr {
            Some(s) => s,
            None => return Err(format_err!("logging is not enabled for this runtime")),
        }
        .reopen()?;

        Ok((BufReader::new(stdout), BufReader::new(stderr)))
    }
}

impl WasiRuntime {
    /// Creates a new WasiRuntime
    ///
    /// # Arguments
    ///
    /// * `module_path` - the path to the WebAssembly binary
    /// * `env` - a collection of key/value pairs containing the environment variables
    /// * `args` - the arguments passed as the command-line arguments list
    /// * `dirs` - a map of local file system paths to optional path names in the runtime
    ///     (e.g. /tmp/foo/myfile -> /app/config). If the optional value is not given,
    ///     the same path will be allowed in the runtime
    /// * `log_file_location` - location for storing logs
    pub fn new<M: AsRef<Path>, L: AsRef<Path> + Copy>(
        module_path: M,
        env: HashMap<String, String>,
        args: Vec<String>,
        dirs: HashMap<String, Option<String>>,
        log_file_location: Option<L>,
    ) -> Result<Self> {
        let module_data = std::fs::read(module_path)?;

        // We need to use named temp file because we need multiple file handles
        // and if we are running in the temp dir, we run the possibility of the
        // temp file getting cleaned out from underneath us while running. If we
        // think it necessary, we can make these permanent files with a cleanup
        // loop that runs elsewhere. These will get deleted when the reference
        // is dropped
        let stdout = match log_file_location {
            Some(l) => Some(NamedTempFile::new_in(l)?),
            None => None,
        };
        let stderr = match log_file_location {
            Some(l) => Some(NamedTempFile::new_in(l)?),
            None => None,
        };

        Ok(WasiRuntime {
            module_data,
            env,
            args,
            dirs,
            stdout,
            stderr,
        })
    }
}
