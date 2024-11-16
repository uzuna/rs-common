use std::net::SocketAddr;

use axum::Router;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;
use tracing_subscriber::prelude::*;

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
    capture_tx: mpsc::Sender<v4l_serve::context::Request>,
}

impl v4l_serve::context::Context for Context {
    fn capture_tx(&self) -> mpsc::Sender<v4l_serve::context::Request> {
        self.capture_tx.clone()
    }
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

    let (mut cap_handle, capture_tx) = v4l_serve::capture::CaptureRoutine::new();
    let token = CancellationToken::new();

    let router = v4l_serve::service::route(Router::new())
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
