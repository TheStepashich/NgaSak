use crate::ngate::traits::NgateBackend;
use anyhow::Result;

pub struct LinuxNgateBackend;

impl NgateBackend for LinuxNgateBackend {
    fn status(&self) -> crate::ngate::traits::BoxFuture<'_, Result<bool>> {
        Box::pin(async move {
            let cfg = crate::config::load_config()?;
            crate::ngate::status::is_online(&cfg.ngate.binary).await
        })
    }

    fn connect(&self) -> crate::ngate::traits::BoxFuture<'_, Result<()>> {
        Box::pin(async move { crate::ngate::dbus::connect().await })
    }

    fn connect_gui_fallback(&self) -> crate::ngate::traits::BoxFuture<'_, Result<()>> {
        Box::pin(async move { crate::ngate::dbus::connect_gui_fallback().await })
    }
}
