#[macro_use]
extern crate failure;

// Tonic will autogenerate the module's body.
pub mod grpc {
    tonic::include_proto!("runtime.v1alpha2");
}

pub mod oci;
pub mod runtime;
pub mod wasm;
