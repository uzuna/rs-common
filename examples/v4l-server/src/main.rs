use std::net::SocketAddr;

use axum::{routing::get, Router};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;
use tracing_subscriber::prelude::*;

mod capture;
mod device;
mod error;
mod util;

#[derive(Clone)]
struct Context {
    capture_tx: mpsc::Sender<capture::Request>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "example_static_file_server=debug,tower_http=debug,info".into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let (mut cap_handle, capture_tx) = capture::CaptureRoutine::new();
    let token = CancellationToken::new();

    let router = Router::new()
        .route("/devices", get(device::list))
        .route("/device/:index", get(device::device))
        .route("/device/:index/capture", get(device::capture))
        .layer(TraceLayer::new_for_http())
        .with_state(Context { capture_tx });

    let port = 8080;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
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
