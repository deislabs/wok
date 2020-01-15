use crate::runtime::Result;
use log::info;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use tempfile::NamedTempFile;
use wasi_common::{preopen_dir, WasiCtxBuilder};
use wasmtime::*;
use wasmtime_wasi::*;

/// EnvVars is a convenience alias around a hash map of String to String
pub type EnvVars = HashMap<String, String>;

/// DirMapping is a convenience alias for a hash map of local file system paths
/// to optional path names in the runtime (e.g. /tmp/foo/myfile -> /app/config).
/// If the optional value is not given, the same path will be allowed in the
/// runtime
pub type DirMapping = HashMap<String, Option<String>>;

/// WasiRuntime provides a WASI compatible runtime. A runtime should be used for
/// each "instance" of a process and can be passed to a thread pool for running
// TODO: Should we have a Trait that this implements along with the WASCC runtime?
pub struct WasiRuntime {
    store: HostRef<Store>,
    module: HostRef<Module>,
    imports: Vec<Extern>,
    stdout: NamedTempFile,
    stderr: NamedTempFile,
}

impl WasiRuntime {
    pub fn new(
        module_path: &str,
        env: EnvVars,
        args: Vec<String>,
        dirs: DirMapping,
        log_file_location: &str,
    ) -> Result<Self> {
        let module_data = std::fs::read(module_path)?;
        let env_vars: Vec<(String, String)> = env.into_iter().collect();
        let engine = HostRef::new(Engine::default());
        let store = HostRef::new(Store::new(&engine));
        let module = HostRef::new(match Module::new(&store, &module_data) {
            Ok(m) => m,
            Err(e) => return Err(format_err!("unable to load module data {}", e)),
        });

        // We need to use named temp file because we need multiple file handles
        // and if we are running in the temp dir, we run the possibility of the
        // temp file getting cleaned out from underneath us while running. If we
        // think it necessary, we can make these permanent files with a cleanup
        // loop that runs elsewhere. These will get deleted when the reference
        // is dropped
        let stdout = NamedTempFile::new_in(log_file_location)?;

        let stderr = NamedTempFile::new_in(log_file_location)?;

        let mut ctx_builder = WasiCtxBuilder::new()
            .args(args)
            .envs(env_vars)
            .stdout(stdout.reopen()?)
            .stderr(stderr.reopen()?);

        for dir in dirs.iter() {
            let guest_dir = match dir.1 {
                Some(s) => s.clone(),
                None => dir.0.clone(),
            };
            // Try and preopen the directory and then try to clone it. This step adds the directory to the context
            ctx_builder = ctx_builder.preopened_dir(preopen_dir(dir.0)?.try_clone()?, guest_dir);
        }

        let wasi_ctx = ctx_builder.build()?;

        // Build the WASI instance and then generate a list of WASI modules
        let global_exports = store.borrow().global_exports().clone();
        let wasi_inst = HostRef::new(wasmtime::Instance::from_handle(
            &store,
            instantiate_wasi_with_context(global_exports, wasi_ctx)?,
        ));
        // Iterate through the module includes and resolve imports
        let imports = module
            .borrow()
            .imports()
            .iter()
            .map(|i| {
                let module_name = i.module().as_str();
                let field_name = i.name().as_str();
                if let Some(export) = wasi_inst.borrow().find_export_by_name(field_name) {
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
        Ok(WasiRuntime {
            store,
            module,
            imports,
            stdout,
            stderr,
        })
    }

    pub fn run(&self) -> Result<()> {
        info!("starting run of module");
        let _instance = match Instance::new(&self.store, &self.module, &self.imports) {
            Ok(i) => i,
            Err(e) => return Err(format_err!("unable to run module: {}", e)),
        };

        info!("module run complete");
        Ok(())
    }

    /// output returns a tuple of BufReaders containing stdout and stderr
    /// respectively. It will error if it can't open a stream
    // TODO(taylor): I can't completely tell from documentation, but we may
    // need to switch this out from a BufReader if it can't handle streaming
    // logs
    pub fn output(&self) -> Result<(BufReader<File>, BufReader<File>)> {
        // As warned in the BufReader docs, creating multiple BufReaders on the
        // same stream can cause data loss. So reopen a new file object each
        // time this function as called so as to not drop any data
        let stdout = self.stdout.reopen()?;
        let stderr = self.stderr.reopen()?;

        Ok((BufReader::new(stdout), BufReader::new(stderr)))
    }
}
