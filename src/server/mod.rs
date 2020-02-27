pub mod image;
pub mod runtime;

// Tonic will autogenerate the module's body.
pub mod grpc {
    tonic::include_proto!("runtime.v1alpha2");
}

pub use grpc::image_service_server::ImageServiceServer;
pub use grpc::runtime_service_server::RuntimeServiceServer;
pub use grpc::Image as Module;

pub use image::CriImageService;
pub use runtime::CriRuntimeService;

/// CriResult describes a Result that has a Response<T> and a Status
pub type CriResult<T> = std::result::Result<tonic::Response<T>, tonic::Status>;
