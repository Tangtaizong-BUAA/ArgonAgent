//! Lightweight secret scanner for local-first safety boundaries.
//!
//! This is not a full DLP engine. It catches common high-risk patterns before
//! cloud model calls, event persistence, or artifact export.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSeverity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretFinding {
    pub kind: String,
    pub severity: SecretSeverity,
    pub preview: String,
}

pub fn scan_text_for_secrets(text: &str) -> Vec<SecretFinding> {
    let mut findings = Vec::new();
    if text.contains("-----BEGIN OPENSSH PRIVATE KEY-----")
        || text.contains("-----BEGIN RSA PRIVATE KEY-----")
    {
        findings.push(SecretFinding {
            kind: "private_key".to_string(),
            severity: SecretSeverity::High,
            preview: "[REDACTED_PRIVATE_KEY]".to_string(),
        });
    }
    for token in text.split(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ',')) {
        if looks_like_openai_key(token) {
            findings.push(SecretFinding {
                kind: "api_key".to_string(),
                severity: SecretSeverity::High,
                preview: "[REDACTED_SECRET]".to_string(),
            });
        } else if looks_like_aws_key(token) {
            findings.push(SecretFinding {
                kind: "aws_access_key".to_string(),
                severity: SecretSeverity::High,
                preview: "[REDACTED_AWS_KEY]".to_string(),
            });
        }
    }
    if text.contains(".env") {
        findings.push(SecretFinding {
            kind: "env_path".to_string(),
            severity: SecretSeverity::Medium,
            preview: "[REDACTED_PATH]".to_string(),
        });
    }
    findings
}

pub fn contains_high_severity_secret(text: &str) -> bool {
    scan_text_for_secrets(text)
        .iter()
        .any(|finding| finding.severity == SecretSeverity::High)
}

pub fn redact_text_for_secrets(text: &str) -> String {
    let without_private_keys = redact_private_key_blocks(text);
    without_private_keys
        .split_whitespace()
        .map(redact_token)
        .collect::<Vec<_>>()
        .join(" ")
        .replace(".env", "[REDACTED_PATH]")
}

fn looks_like_openai_key(token: &str) -> bool {
    token.starts_with("sk-") && token.len() >= 16
}

fn looks_like_aws_key(token: &str) -> bool {
    token.len() == 20
        && (token.starts_with("AKIA") || token.starts_with("ASIA"))
        && token
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
}

fn redact_private_key_blocks(text: &str) -> String {
    let mut redacted = String::new();
    let mut in_private_key = false;
    for line in text.lines() {
        if line.contains("-----BEGIN OPENSSH PRIVATE KEY-----")
            || line.contains("-----BEGIN RSA PRIVATE KEY-----")
        {
            in_private_key = true;
            redacted.push_str("[REDACTED_PRIVATE_KEY]\n");
            continue;
        }
        if in_private_key {
            if line.contains("-----END OPENSSH PRIVATE KEY-----")
                || line.contains("-----END RSA PRIVATE KEY-----")
            {
                in_private_key = false;
            }
            continue;
        }
        redacted.push_str(line);
        redacted.push('\n');
    }
    if text.ends_with('\n') {
        redacted
    } else {
        redacted.trim_end_matches('\n').to_string()
    }
}

fn redact_token(token: &str) -> String {
    let trimmed = token.trim_matches(|ch: char| matches!(ch, '"' | '\'' | ',' | ';'));
    if looks_like_openai_key(trimmed) {
        token.replace(trimmed, "[REDACTED_SECRET]")
    } else if looks_like_aws_key(trimmed) {
        token.replace(trimmed, "[REDACTED_AWS_KEY]")
    } else {
        token.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_secret_patterns_without_echoing_value() {
        let findings =
            scan_text_for_secrets("key sk-testsecret123456789 in .env and AKIA1234567890ABCDEF");
        assert!(findings.iter().any(|finding| finding.kind == "api_key"));
        assert!(findings.iter().any(|finding| finding.kind == "env_path"));
        assert!(findings
            .iter()
            .all(|finding| !finding.preview.contains("sk-testsecret")));
        assert!(contains_high_severity_secret(
            "-----BEGIN OPENSSH PRIVATE KEY-----"
        ));
    }

    #[test]
    fn benign_text_has_no_findings() {
        assert!(scan_text_for_secrets("visible assistant answer").is_empty());
    }

    #[test]
    fn redacts_secret_values_without_preserving_token() {
        let text = "stdout sk-testsecret123456789 .env AKIA1234567890ABCDEF";
        let redacted = redact_text_for_secrets(text);
        assert!(redacted.contains("[REDACTED_SECRET]"));
        assert!(redacted.contains("[REDACTED_PATH]"));
        assert!(redacted.contains("[REDACTED_AWS_KEY]"));
        assert!(!redacted.contains("sk-testsecret"));
        assert!(!redacted.contains("AKIA123"));
    }
}
