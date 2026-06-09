//! Local artifact store primitives.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactKind {
    Diff,
    CommandOutput,
    Chart,
    Report,
    Notebook,
    Script,
    DataProfile,
    Dataset,
    Manifest,
    Log,
    ModelTranscript,
    ToolResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRecord {
    pub artifact_id: String,
    pub kind: ArtifactKind,
    pub content_hash: String,
    pub size_bytes: u64,
    pub privacy_class: String,
    pub relative_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn put_bytes(
        &self,
        artifact_id: impl Into<String>,
        kind: ArtifactKind,
        content_hash: impl Into<String>,
        privacy_class: impl Into<String>,
        bytes: &[u8],
    ) -> Result<ArtifactRecord, io::Error> {
        let artifact_id = artifact_id.into();
        let content_hash = content_hash.into();
        let shard = content_hash.get(0..2).unwrap_or("00");
        let relative_path = PathBuf::from("sha256").join(shard).join(&content_hash);
        let full_path = self.root.join(&relative_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full_path, bytes)?;
        Ok(ArtifactRecord {
            artifact_id,
            kind,
            content_hash,
            size_bytes: bytes.len() as u64,
            privacy_class: privacy_class.into(),
            relative_path,
        })
    }

    pub fn put_bytes_auto_hash(
        &self,
        artifact_id: impl Into<String>,
        kind: ArtifactKind,
        privacy_class: impl Into<String>,
        bytes: &[u8],
    ) -> Result<ArtifactRecord, io::Error> {
        self.put_bytes(
            artifact_id,
            kind,
            stable_content_hash(bytes),
            privacy_class,
            bytes,
        )
    }

    pub fn read_bytes(&self, record: &ArtifactRecord) -> Result<Vec<u8>, io::Error> {
        fs::read(self.root.join(&record.relative_path))
    }

    pub fn write_manifest(&self, records: &[ArtifactRecord]) -> Result<PathBuf, io::Error> {
        let manifest_path = self.root.join("manifest.json");
        fs::create_dir_all(&self.root)?;
        fs::write(&manifest_path, manifest_json(records))?;
        Ok(manifest_path)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

pub fn manifest_json(records: &[ArtifactRecord]) -> String {
    let mut output = String::from("{\"schema_version\":\"artifact_manifest.v0\",\"artifacts\":[");
    for (index, record) in records.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(&record_json(record));
    }
    output.push_str("]}");
    output
}

fn record_json(record: &ArtifactRecord) -> String {
    format!(
        "{{\"artifact_id\":\"{}\",\"kind\":\"{}\",\"content_hash\":\"{}\",\"size_bytes\":{},\"privacy_class\":\"{}\",\"relative_path\":\"{}\"}}",
        escape(&record.artifact_id),
        artifact_kind_to_str(&record.kind),
        escape(&record.content_hash),
        record.size_bytes,
        escape(&record.privacy_class),
        escape(&record.relative_path.to_string_lossy())
    )
}

pub fn artifact_kind_to_str(kind: &ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Diff => "diff",
        ArtifactKind::CommandOutput => "command_output",
        ArtifactKind::Chart => "chart",
        ArtifactKind::Report => "report",
        ArtifactKind::Notebook => "notebook",
        ArtifactKind::Script => "script",
        ArtifactKind::DataProfile => "data_profile",
        ArtifactKind::Dataset => "dataset",
        ArtifactKind::Manifest => "manifest",
        ArtifactKind::Log => "log",
        ArtifactKind::ModelTranscript => "model_transcript",
        ArtifactKind::ToolResult => "tool_result",
    }
}

pub fn stable_content_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv64_{hash:016x}")
}

fn escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            other if other.is_control() => format!("\\u{:04x}", other as u32).chars().collect(),
            other => vec![other],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn writes_and_reads_artifact() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-artifacts-{nonce}"));
        let store = ArtifactStore::new(&root);
        let record = store
            .put_bytes(
                "artifact_1",
                ArtifactKind::CommandOutput,
                "abcdef123456",
                "internal",
                b"hello",
            )
            .unwrap();
        assert_eq!(record.size_bytes, 5);
        assert_eq!(store.read_bytes(&record).unwrap(), b"hello");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn writes_manifest_index() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-artifacts-manifest-{nonce}"));
        let store = ArtifactStore::new(&root);
        let record = store
            .put_bytes_auto_hash("artifact_1", ArtifactKind::Report, "internal", b"# Report")
            .unwrap();
        let manifest_path = store.write_manifest(&[record]).unwrap();
        let manifest = fs::read_to_string(&manifest_path).unwrap();
        assert!(manifest.contains("\"schema_version\":\"artifact_manifest.v0\""));
        assert!(manifest.contains("\"kind\":\"report\""));
        assert!(manifest.contains("\"content_hash\":\"fnv64_"));
        let _ = fs::remove_dir_all(root);
    }
}
