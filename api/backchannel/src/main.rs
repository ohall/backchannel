use backchannel_core::{create_router, db, Config};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::OnceCell;
use tower::Service;
use tower::ServiceExt;
use vercel_runtime::{run, AppState, Error, ResponseBody};

type Request = hyper::Request<hyper::body::Incoming>;
type Response = hyper::Response<ResponseBody>;

static ROUTER: OnceCell<axum::Router> = OnceCell::const_new();

/// Lazily build the router (and DB pool) on first request.
///
/// This must be async: the handler already runs inside the Tokio runtime, so
/// blocking on a future here (e.g. `Handle::block_on`) would panic with
/// "Cannot start a runtime from within a runtime". A failed init is not
/// cached, so the next request retries (e.g. after a Supabase cold start).
async fn get_router() -> Result<&'static axum::Router, Error> {
    ROUTER
        .get_or_try_init(|| async {
            let config =
                Config::from_env().map_err(|e| Error::from(format!("Config error: {}", e)))?;

            let pool = db::create_pool(&config.database_url, &config.database_schema)
                .await
                .map_err(|e| Error::from(format!("Database connection error: {}", e)))?;

            Ok::<_, Error>(create_router(pool, config))
        })
        .await
}

#[derive(Clone)]
struct BackchannelService;

impl Service<(AppState, Request)> for BackchannelService {
    type Response = Response;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, (_state, req): (AppState, Request)) -> Self::Future {
        Box::pin(async move {
            let router = get_router().await?;

            let response = router
                .clone()
                .oneshot(req)
                .await
                .map_err(|e| Error::from(format!("Router error: {}", e)))?;

            // Convert axum response to vercel response
            let (parts, body) = response.into_parts();
            let body_bytes = axum::body::to_bytes(body, usize::MAX)
                .await
                .map_err(|e| Error::from(format!("Body error: {}", e)))?;

            let mut vercel_response = hyper::Response::new(ResponseBody::from(body_bytes.to_vec()));
            *vercel_response.status_mut() = parts.status;
            *vercel_response.headers_mut() = parts.headers;
            *vercel_response.version_mut() = parts.version;

            Ok(vercel_response)
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(BackchannelService).await
}
