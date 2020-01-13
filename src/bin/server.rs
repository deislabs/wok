use tonic::transport::Server;
use wok::grpc::runtime_service_server::RuntimeServiceServer;
use wok::CRIRuntimeService;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "127.0.0.1:50051".parse()?;
    let runtime = CRIRuntimeService::default();
    env_logger::init();

    log::info!("listening on {}", addr);

    Server::builder()
        .add_service(RuntimeServiceServer::new(runtime))
        .serve(addr)
        .await?;

    Ok(())
}
