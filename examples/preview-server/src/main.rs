use std::net::SocketAddr;

use axum::Router;
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing_subscriber::prelude::*;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "example_static_file_server=debug,tower_http=debug,info".into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let serve_dir = ServeDir::new("assets").append_index_html_on_directories(true);
    let router = Router::new()
        .fallback_service(serve_dir)
        .layer(TraceLayer::new_for_http());

    let port = 8080;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, router).await.unwrap();
}
