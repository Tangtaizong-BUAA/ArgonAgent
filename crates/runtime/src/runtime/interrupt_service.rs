use crate::live_http_transport::LiveHttpTransport;
use crate::native_agent_loop::{NativeAgentLoopResult, NativeAgentLoopV2Request};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug)]
pub struct InterruptService {
    flag: Arc<AtomicBool>,
}

impl Default for InterruptService {
    fn default() -> Self {
        Self::new()
    }
}

impl InterruptService {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn handle(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }

    pub fn interrupt(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    pub fn reset(&self) {
        self.flag.store(false, Ordering::Relaxed);
    }

    pub fn is_interrupted(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    pub fn run_deepseek_agent_loop_request_with_interrupt<T: LiveHttpTransport>(
        transport: &T,
        request: NativeAgentLoopV2Request,
        event_sink: Option<&mut dyn FnMut(&str)>,
        interrupt: &AtomicBool,
    ) -> Result<NativeAgentLoopResult, String> {
        crate::native_agent_loop::run_native_agent_loop_v2_deepseek_with_interrupt(
            transport, request, event_sink, interrupt,
        )
    }
}
