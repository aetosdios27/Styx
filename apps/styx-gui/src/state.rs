use tokio::sync::Mutex;

use styx_app::MemoryRuntime;

#[derive(Debug, Default)]
pub struct GuiState {
    runtime: Mutex<MemoryRuntime>,
}

impl GuiState {
    pub async fn with_runtime<T>(&self, f: impl FnOnce(&mut MemoryRuntime) -> T) -> T {
        let mut runtime = self.runtime.lock().await;
        f(&mut runtime)
    }
}
