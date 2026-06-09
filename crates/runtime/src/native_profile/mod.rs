//! Native model profile extraction target.

pub mod deepseek;
pub mod qwen;

use researchcode_kernel::model::NativeModelFamily;

pub trait NativeProfile {
    fn family(&self) -> NativeModelFamily;
    fn profile_name(&self) -> &'static str;
    fn supports_reasoning_replay(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub enum NativeProfileInstance {
    DeepSeek(deepseek::DeepSeekProfile),
    Qwen(qwen::QwenProfile),
}

impl NativeProfile for NativeProfileInstance {
    fn family(&self) -> NativeModelFamily {
        match self {
            NativeProfileInstance::DeepSeek(profile) => profile.family(),
            NativeProfileInstance::Qwen(profile) => profile.family(),
        }
    }

    fn profile_name(&self) -> &'static str {
        match self {
            NativeProfileInstance::DeepSeek(profile) => profile.profile_name(),
            NativeProfileInstance::Qwen(profile) => profile.profile_name(),
        }
    }

    fn supports_reasoning_replay(&self) -> bool {
        match self {
            NativeProfileInstance::DeepSeek(profile) => profile.supports_reasoning_replay(),
            NativeProfileInstance::Qwen(profile) => profile.supports_reasoning_replay(),
        }
    }
}

pub fn profile_for_family(family: NativeModelFamily) -> NativeProfileInstance {
    match family {
        NativeModelFamily::DeepSeek => {
            NativeProfileInstance::DeepSeek(deepseek::DeepSeekProfile::default())
        }
        NativeModelFamily::Qwen => NativeProfileInstance::Qwen(qwen::QwenProfile::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_returns_deepseek_native_profile() {
        let profile = profile_for_family(NativeModelFamily::DeepSeek);
        assert_eq!(profile.family(), NativeModelFamily::DeepSeek);
        assert!(profile.supports_reasoning_replay());
    }

    #[test]
    fn factory_returns_qwen_native_profile() {
        let profile = profile_for_family(NativeModelFamily::Qwen);
        assert_eq!(profile.family(), NativeModelFamily::Qwen);
        assert!(profile.supports_reasoning_replay());
    }
}
