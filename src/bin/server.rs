use tonic::transport::Server;

use wok::CRIRuntimeService;
use wok::grpc::runtime_service_server::RuntimeServiceServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;
    let runtime = CRIRuntimeService::default();
    env_logger::init();

    log::info!("listening on {}", addr);

    Server::builder()
        .add_service(RuntimeServiceServer::new(runtime))
        .serve(addr)
        .await?;

    Ok(())
}
