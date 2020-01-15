use tonic::{Response, Status};
#[macro_use]
extern crate failure;

pub mod wasm;
// TODO(taylor): Depending on how things shake out, we could remove the pub from
// runtime and just export the specific structs
pub mod runtime;
pub use runtime::{CRIRuntimeService, RUNTIME_API_VERSION};

// Tonic will autogenerate the module's body.
pub mod grpc {
    tonic::include_proto!("runtime.v1alpha2");
}

pub mod oci;

type CriResult<T> = std::result::Result<Response<T>, Status>;

type Result<T> = std::result::Result<T, failure::Error>;
