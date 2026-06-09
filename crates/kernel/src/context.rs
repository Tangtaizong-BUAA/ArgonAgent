//! ContextBundle primitives for model calls.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextItemKind {
    UserTask,
    ProjectInstructions,
    Plan,
    RepoMap,
    FileSnippet,
    SearchResult,
    GitStatus,
    ToolResultPreview,
    ResearchProfile,
    PrivacyReport,
    Memory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextItem {
    pub item_id: String,
    pub kind: ContextItemKind,
    pub source: String,
    pub content: String,
    pub token_estimate: u64,
    pub privacy_class: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextBundle {
    pub bundle_id: String,
    pub model_family: String,
    pub max_context_tokens: u64,
    pub items: Vec<ContextItem>,
}

impl ContextBundle {
    pub fn token_estimate(&self) -> u64 {
        self.items.iter().map(|item| item.token_estimate).sum()
    }

    pub fn push_if_fits(&mut self, item: ContextItem) -> bool {
        if self.token_estimate() + item.token_estimate > self.max_context_tokens {
            return false;
        }
        self.items.push(item);
        true
    }
}

pub fn estimate_tokens(text: &str) -> u64 {
    (text.len() as u64 / 4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_if_fits_enforces_budget() {
        let mut bundle = ContextBundle {
            bundle_id: "bundle".to_string(),
            model_family: "qwen".to_string(),
            max_context_tokens: 2,
            items: Vec::new(),
        };
        assert!(bundle.push_if_fits(ContextItem {
            item_id: "item_1".to_string(),
            kind: ContextItemKind::UserTask,
            source: "user".to_string(),
            content: "hello".to_string(),
            token_estimate: 1,
            privacy_class: "internal".to_string(),
        }));
        assert!(!bundle.push_if_fits(ContextItem {
            item_id: "item_2".to_string(),
            kind: ContextItemKind::FileSnippet,
            source: "README.md".to_string(),
            content: "too large".to_string(),
            token_estimate: 2,
            privacy_class: "internal".to_string(),
        }));
    }
}
