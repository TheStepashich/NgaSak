use anyhow::Result;
use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait NgateBackend: Send + Sync {
    fn status(&self) -> BoxFuture<'_, Result<bool>>;
    fn connect(&self) -> BoxFuture<'_, Result<()>>;
    fn connect_gui_fallback(&self) -> BoxFuture<'_, Result<()>>;
}
