use std::error;
use std::fmt;
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

#[cfg(unix)]
use futures::stream::TryStreamExt;
#[cfg(unix)]
use tokio::net::UnixListener;
use tonic::transport::Server;

use ipnet::IpNet;
#[cfg(unix)]
use wok::server::{CriImageService, CriRuntimeService, ImageServiceServer, RuntimeServiceServer};

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

#[derive(clap::Clap)]
struct Opts {
    #[clap(short = "a", long = "addr", default_value = "unix:///tmp/wok.sock")]
    addr: String,

    #[clap(short = "d", long = "dir", default_value = "/tmp")]
    dir: PathBuf,

    #[clap(long = "pod-cidr")]
    pod_cidr: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let opts: Opts = Opts::parse();
    let pod_cidr = match opts.pod_cidr {
        Some(s) => Some(IpNet::from_str(&s)?),
        None => None,
    };
    log::debug!("Using {:?} for pod CIDR", pod_cidr);
    let runtime = CriRuntimeService::new(opts.dir.clone(), pod_cidr).await;
    let image_service = CriImageService::new(opts.dir.clone()).await;

    let parts: Vec<&str> = opts.addr.split("://").collect();

    if parts.len() != 2 {
        return Err(BadAddr.into());
    }

    log::info!("listening on {}", parts[1]);

    // Temporary work-around for async/.await
    serve(parts[0], parts[1], runtime, image_service)
        .await
        .map(|_| ())
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

/// Create a server for handling CRI Runtime requests.
#[cfg(unix)]
async fn serve(
    proto: &str,
    addr: &str,
    runtime: CriRuntimeService,
    image_service: CriImageService,
) -> Result<(), Box<dyn std::error::Error>> {
    match proto {
        "unix" => {
            // attempt to create base directory if it doesn't already exist
            tokio::fs::create_dir_all(Path::new(addr).parent().unwrap_or_else(|| Path::new(addr)))
                .await?;

            let mut uds = UnixListener::bind(addr)?;

            let path = addr.to_owned();
            ctrlc::set_handler(move || {
                // ignore the error if we fail to remove the file; there can be cases where the user exits before the UDS is bound
                fs::remove_file(&path).unwrap_or(());
                std::process::exit(0);
            })
            .expect("Error setting exit handler");

            Server::builder()
                .add_service(RuntimeServiceServer::new(runtime))
                .add_service(ImageServiceServer::new(image_service))
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
    runtime: CriRuntimeService,
    _image_service: CriImageService,
) -> Result<(), Box<dyn std::error::Error>> {
    match proto {
        "unix" => {
            panic!("unix domain sockets are not supported on Windows!");
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
