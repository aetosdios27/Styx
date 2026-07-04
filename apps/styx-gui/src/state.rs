use tokio::sync::Mutex;

use styx_app::AppError;
use styx_runtime::{AppRuntime, RuntimeConfig, RuntimeEngine};

#[derive(Debug)]
pub struct GuiState {
    runtime: Mutex<AppRuntime>,
}

impl GuiState {
    pub fn new(listen_port: u16) -> Result<Self, AppError> {
        let config = RuntimeConfig {
            listen_port,
            ..RuntimeConfig::default()
        };
        let engine =
            RuntimeEngine::new(config).map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(Self {
            runtime: Mutex::new(AppRuntime::new(engine)),
        })
    }

    pub async fn with_runtime<T>(&self, f: impl FnOnce(&mut AppRuntime) -> T) -> T {
        let mut runtime = self.runtime.lock().await;
        f(&mut runtime)
    }
}
