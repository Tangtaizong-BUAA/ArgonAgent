pub mod adaptation;
pub mod cache_prefix;
pub mod policy;
pub mod reasoning;
pub mod role_split;
pub mod stream;
pub mod stream_processor;

use researchcode_kernel::model::NativeModelFamily;

use crate::native_profile::NativeProfile;

#[derive(Debug, Default, Clone, Copy)]
pub struct DeepSeekProfile;

impl NativeProfile for DeepSeekProfile {
    fn family(&self) -> NativeModelFamily {
        NativeModelFamily::DeepSeek
    }

    fn profile_name(&self) -> &'static str {
        "deepseek-native"
    }

    fn supports_reasoning_replay(&self) -> bool {
        true
    }
}
