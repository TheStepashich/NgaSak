pub mod traits;
pub mod status;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub mod dbus;
#[cfg(target_os = "linux")]
pub mod gui;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "linux")]
pub use linux as backend;
#[cfg(target_os = "windows")]
pub use windows as backend;

use crate::config::Config;
use crate::ngate::traits::NgateBackend;
use std::sync::Arc;

pub fn create_backend(_cfg: &Config) -> Arc<dyn NgateBackend> {
    #[cfg(target_os = "linux")]
    {
        Arc::new(linux::LinuxNgateBackend)
    }
    #[cfg(target_os = "windows")]
    {
        Arc::new(windows::WindowsNgateBackend::new())
    }
}
