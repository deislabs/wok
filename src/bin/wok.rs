use std::error;
use std::fmt;
use std::path::Path;

use futures::stream::TryStreamExt;

#[cfg(unix)]
use tokio::net::UnixListener;

#[macro_use]
extern crate clap;
use tonic::transport::Server;

use wok::{grpc::runtime_service_server::RuntimeServiceServer, runtime::CRIRuntimeService};

#[derive(Debug, Clone)]
struct BadAddr;

impl fmt::Display for BadAddr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "invalid address")
    }
}

impl error::Error for BadAddr {
    fn description(&self) -> &str {
        "invalid address"
    }

    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        // Generic error, underlying cause isn't tracked.
        None
    }
}

#[derive(Clap)]
struct Opts {
    #[clap(short = "a", long = "addr", default_value = "unix:///tmp/wok.sock")]
    addr: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opts: Opts = Opts::parse();
    let runtime = CRIRuntimeService::new();

    env_logger::init();

    let parts: Vec<&str> = opts.addr.split("://").collect();

    if parts.len() != 2 {
        return Err(BadAddr.into());
    }

    let proto = parts[0];
    let addr = parts[1];

    log::info!("listening on {}", addr);

    // Temporary work-around for async/.await
    match serve(proto, addr, runtime).await {
        Err(e) => Err(e),
        _ => Ok(()),
    }
}

#[cfg(unix)]
mod unix {
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };

    use tokio::io::{AsyncRead, AsyncWrite};
    use tonic::transport::server::Connected;

    #[derive(Debug)]
    pub struct UnixStream(pub tokio::net::UnixStream);

    impl Connected for UnixStream {}

    impl AsyncRead for UnixStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.0).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for UnixStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.0).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_shutdown(cx)
        }
    }
}

#[cfg(unix)]
async fn serve(
    proto: &str,
    addr: &str,
    runtime: CRIRuntimeService,
) -> Result<(), Box<dyn std::error::Error>> {
    match proto {
        "unix" => {
            // attempt to create base directory if it doesn't already exist
            tokio::fs::create_dir_all(Path::new(addr).parent().unwrap_or_else(|| Path::new(addr)))
                .await?;

            let mut uds = UnixListener::bind(addr)?;

            Server::builder()
                .add_service(RuntimeServiceServer::new(runtime))
                .serve_with_incoming(uds.incoming().map_ok(unix::UnixStream))
                .await?;
        }
        "tcp" => {
            let listener = addr.parse::<std::net::SocketAddr>()?;

            Server::builder()
                .add_service(RuntimeServiceServer::new(runtime))
                .serve(listener)
                .await?;
        }
        _ => return Err(BadAddr.into()),
    }

    Ok(())
}

#[cfg(windows)]
async fn serve(
    proto: &str,
    addr: &str,
    runtime: CRIRuntimeService,
) -> Result<(), Box<dyn std::error::Error>> {
    match proto {
        "unix" => {
            panic!("unix domain sockets are not supported on Windows!");
        }
        "tcp" => {
            let listener = tokio::net::TcpListener::bind(addr).await?;

            Server::builder()
                .add_service(RuntimeServiceServer::new(runtime))
                .serve(listener)
                .await?;
        }
        _ => return Err(BadAddr.into()),
    }

    Ok(())
}
