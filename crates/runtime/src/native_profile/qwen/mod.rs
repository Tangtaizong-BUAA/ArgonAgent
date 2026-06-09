pub mod budget;
pub mod cache_prefix;
pub mod reasoning;
pub mod stream_processor;

use researchcode_kernel::model::NativeModelFamily;

use crate::native_profile::NativeProfile;

#[derive(Debug, Default, Clone, Copy)]
pub struct QwenProfile;

impl NativeProfile for QwenProfile {
    fn family(&self) -> NativeModelFamily {
        NativeModelFamily::Qwen
    }

    fn profile_name(&self) -> &'static str {
        "qwen-native"
    }

    fn supports_reasoning_replay(&self) -> bool {
        true
    }
}
