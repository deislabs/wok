use wok::grpc::runtime_service_client::RuntimeServiceClient;
use wok::grpc::VersionRequest;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = RuntimeServiceClient::connect("http://[::1]:50051").await?;

    let request = tonic::Request::new(VersionRequest {
        version: "1.0.0".into(),
    });

    let response = client.version(request).await?;

    println!("RESPONSE={:?}", response);

    Ok(())
}
