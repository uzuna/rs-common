use std::net::SocketAddr;

use axum::{routing::get, Router};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;
use tracing_subscriber::prelude::*;

mod capture;
mod device;
mod error;
mod imgfmt;
mod util;

#[derive(Debug, clap::Parser)]
struct Opt {
    #[arg(short, long, default_value = "0.0.0.0")]
    addr: String,
    #[arg(short, long, default_value = "8080")]
    port: u16,
}

impl Opt {
    fn addr(&self) -> anyhow::Result<SocketAddr> {
        let addr: SocketAddr = format!("{}:{}", self.addr, self.port).parse()?;
        Ok(addr)
    }
}

#[derive(Clone)]
struct Context {
    capture_tx: mpsc::Sender<capture::Request>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let opt = <Opt as clap::Parser>::parse();

    let (mut cap_handle, capture_tx) = capture::CaptureRoutine::new();
    let token = CancellationToken::new();

    let router = Router::new()
        .route("/devices", get(device::list))
        .route("/device/:index", get(device::device))
        .route("/device/:index/capture", get(device::capture))
        .layer(TraceLayer::new_for_http())
        .with_state(Context { capture_tx });

    let listener = tokio::net::TcpListener::bind(opt.addr()?).await?;
    tracing::info!("listening on {}", listener.local_addr()?);
    let token_clone = token.clone();
    tokio::try_join!(
        async {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    token_clone.cancelled().await;
                })
                .await?;
            Ok(())
        },
        cap_handle.start(token)
    )?;
    Ok(())
}
