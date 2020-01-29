#[macro_use]
extern crate failure;

// Tonic will autogenerate the module's body.
pub mod grpc {
    tonic::include_proto!("runtime.v1alpha2");
}

pub mod oci;
pub mod runtime;
mod util;
pub mod wasm;

pub use grpc::image_service_server::ImageServiceServer;
pub use grpc::runtime_service_server::RuntimeServiceServer;
pub use oci::CriImageService;
pub use runtime::{CriResult, CriRuntimeService};
