//! Memory primitives for local-first long-task continuity.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryScope {
    Project,
    UserPreference,
    ModelFailure,
    RepoFact,
    ResearchProject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryItem {
    pub memory_id: String,
    pub scope: MemoryScope,
    pub source: String,
    pub content: String,
    pub privacy_class: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryValidationError {
    MissingMemoryId,
    MissingSource,
    MissingContent,
    MissingContentHash,
    SecretLikeContent,
}

impl MemoryItem {
    pub fn validate(&self) -> Result<(), MemoryValidationError> {
        if self.memory_id.trim().is_empty() {
            return Err(MemoryValidationError::MissingMemoryId);
        }
        if self.source.trim().is_empty() {
            return Err(MemoryValidationError::MissingSource);
        }
        if self.content.trim().is_empty() {
            return Err(MemoryValidationError::MissingContent);
        }
        if self.content_hash.trim().is_empty() {
            return Err(MemoryValidationError::MissingContentHash);
        }
        if contains_secret_like_content(&self.content) {
            return Err(MemoryValidationError::SecretLikeContent);
        }
        Ok(())
    }

    pub fn to_context_text(&self) -> String {
        format!(
            "memory_id={} scope={} source={}\n{}",
            self.memory_id,
            memory_scope_to_str(&self.scope),
            self.source,
            self.content
        )
    }
}

pub fn memory_scope_to_str(scope: &MemoryScope) -> &'static str {
    match scope {
        MemoryScope::Project => "project",
        MemoryScope::UserPreference => "user_preference",
        MemoryScope::ModelFailure => "model_failure",
        MemoryScope::RepoFact => "repo_fact",
        MemoryScope::ResearchProject => "research_project",
    }
}

fn contains_secret_like_content(content: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    lowered.contains("sk-")
        || lowered.contains("api_key")
        || lowered.contains("private key")
        || lowered.contains(".env")
        || lowered.contains("id_rsa")
        || lowered.contains("id_ed25519")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_memory_item_and_context_text() {
        let memory = MemoryItem {
            memory_id: "mem_1".to_string(),
            scope: MemoryScope::ModelFailure,
            source: "eval/qwen/parser".to_string(),
            content: "Qwen executor must include base_hash before patch.apply.".to_string(),
            privacy_class: "internal".to_string(),
            content_hash: "fnv64_test".to_string(),
        };
        assert_eq!(memory.validate(), Ok(()));
        assert!(memory.to_context_text().contains("model_failure"));
    }

    #[test]
    fn rejects_secret_like_memory_content() {
        let memory = MemoryItem {
            memory_id: "mem_1".to_string(),
            scope: MemoryScope::Project,
            source: "note".to_string(),
            content: "API_KEY=sk-secret".to_string(),
            privacy_class: "secret".to_string(),
            content_hash: "fnv64_secret".to_string(),
        };
        assert_eq!(
            memory.validate(),
            Err(MemoryValidationError::SecretLikeContent)
        );
    }
}
