//! The connection to `carbide_bridge`, the app's whole data API.
//!
//! The bridge is async and runs on its own tokio runtime. A page never blocks the GTK main
//! thread: it starts the request with [`run`] inside `glib::spawn_future_local`, awaits
//! the answer there, and then changes its widgets on the main thread.

use std::future::Future;
use std::sync::{Arc, OnceLock};

pub use carbide_bridge::api;

pub type Explorer = Arc<api::Explorer>;

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// The runtime of the bridge: worker threads that do the requests.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("bridge")
            .enable_all()
            .build()
            .expect("the bridge runtime did not start")
    })
}

/// Runs `work` on the bridge runtime. Await the result on the main loop.
///
/// ```ignore
/// let ex = explorer.clone();
/// glib::spawn_future_local(async move {
///     let status = bridge::run(async move { ex.chain_status().await }).await;
///     // Change widgets here. This is the main thread.
/// });
/// ```
pub async fn run<T: Send + 'static>(work: impl Future<Output = T> + Send + 'static) -> T {
    match runtime().spawn(work).await {
        Ok(value) => value,
        Err(error) => std::panic::resume_unwind(error.into_panic()),
    }
}

/// Runs a function that blocks, such as opening the configuration files, off the main thread.
pub async fn run_blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    match runtime().spawn_blocking(work).await {
        Ok(value) => value,
        Err(error) => std::panic::resume_unwind(error.into_panic()),
    }
}

/// The text of a bridge error, as the Swift app's `describe`.
pub fn describe(error: &api::CarbideError) -> String {
    match error {
        api::CarbideError::Failed { message } => message.clone(),
    }
}
