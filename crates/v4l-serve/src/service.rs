use axum::{routing::get, Router};

use crate::{context::Context, device};

/// Routerの作成
pub fn route<C>(router: Router<C>) -> Router<C>
where
    C: Context + Clone + Send + Sync + 'static,
{
    router
        .route("/devices", get(device::list))
        .route("/device/:index", get(device::device))
        .route("/device/:index/capture", get(device::capture::<C>))
}
