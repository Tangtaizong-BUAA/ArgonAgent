//! Safe local file read tool.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReadRequest {
    pub path: PathBuf,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReadResult {
    pub path: PathBuf,
    pub content: String,
    pub truncated: bool,
    pub size_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileToolError {
    SensitivePath(String),
    PathEscapesWorkspace(String),
    Io(String),
    Utf8(String),
}

pub fn read_file(
    request: &FileReadRequest,
    workspace_root: &Path,
) -> Result<FileReadResult, FileToolError> {
    let path_text = request.path.to_string_lossy();
    if is_sensitive_path(&path_text) {
        return Err(FileToolError::SensitivePath(path_text.to_string()));
    }
    // Validate the target path is within workspace_root.
    let ws_root = workspace_root
        .canonicalize()
        .map_err(|e| FileToolError::Io(format!("cannot canonicalize workspace_root: {e}")))?;
    let resolved_target = if request.path.exists() {
        request
            .path
            .canonicalize()
            .map_err(|e| FileToolError::Io(format!("cannot resolve path: {e}")))?
    } else {
        // For new files that don't exist yet, canonicalize the parent.
        let parent = request.path.parent().unwrap_or(Path::new("."));
        let resolved_parent = parent
            .canonicalize()
            .map_err(|e| FileToolError::Io(format!("cannot resolve parent: {e}")))?;
        if !resolved_parent.starts_with(&ws_root) {
            return Err(FileToolError::PathEscapesWorkspace(path_text.to_string()));
        }
        resolved_parent.join(request.path.file_name().unwrap_or_default())
    };
    if !resolved_target.starts_with(&ws_root) {
        return Err(FileToolError::PathEscapesWorkspace(path_text.to_string()));
    }
    // Stream read to avoid OOM: use File::open + .take(max_bytes).
    let total_size = std::fs::metadata(&resolved_target)
        .map_err(io_error)?
        .len()
        .min(usize::MAX as u64) as usize;
    let mut file = File::open(&resolved_target).map_err(io_error)?;
    let max_bytes = request.max_bytes.max(1);
    let mut buffer = Vec::with_capacity(max_bytes);
    let bytes_read = file
        .by_ref()
        .take(max_bytes as u64 + 1) // read one extra byte to detect truncation
        .read_to_end(&mut buffer)
        .map_err(io_error)?;
    let truncated = total_size > max_bytes || bytes_read > max_bytes;
    if truncated {
        buffer.truncate(max_bytes);
    }
    let content = match std::str::from_utf8(&buffer) {
        Ok(content) => content.to_string(),
        Err(error) if truncated && error.error_len().is_none() => {
            buffer.truncate(error.valid_up_to());
            std::str::from_utf8(&buffer)
                .map_err(|error| FileToolError::Utf8(error.to_string()))?
                .to_string()
        }
        Err(error) => return Err(FileToolError::Utf8(error.to_string())),
    };
    Ok(FileReadResult {
        path: request.path.clone(),
        content,
        truncated,
        size_bytes: total_size,
    })
}

pub fn is_sensitive_path(path: &str) -> bool {
    let lowered = path.to_lowercase();
    lowered.ends_with(".env")
        || lowered.contains("/.env")
        || lowered.contains("id_rsa")
        || lowered.contains("id_ed25519")
        || lowered.contains(".ssh")
        || lowered.contains("private_key")
        || lowered.ends_with(".pem")
        || lowered.ends_with(".key")
        || lowered.ends_with(".pfx")
        || lowered.contains("credentials")
        || lowered.contains("token")
        || lowered.contains("/proc/")
        || lowered.contains("/dev/fd/")
        || lowered.starts_with("/proc/")
        || lowered.starts_with("/dev/fd/")
}

fn io_error(error: io::Error) -> FileToolError {
    FileToolError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn reads_file_with_truncation_flag() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-file-read-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("test.txt");
        fs::write(&path, "abcdef").unwrap();
        let result = read_file(
            &FileReadRequest {
                path: path.clone(),
                max_bytes: 3,
            },
            &root,
        )
        .unwrap();
        assert_eq!(result.content, "abc");
        assert!(result.truncated);
        assert_eq!(result.size_bytes, 6);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn truncation_keeps_valid_utf8_boundary() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-file-read-utf8-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("test.txt");
        fs::write(&path, "我abc").unwrap();
        let result = read_file(
            &FileReadRequest {
                path: path.clone(),
                max_bytes: 2,
            },
            &root,
        )
        .unwrap();
        assert_eq!(result.content, "");
        assert!(result.truncated);
        assert_eq!(result.size_bytes, "我abc".len());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_sensitive_paths() {
        let root = std::env::temp_dir();
        let result = read_file(
            &FileReadRequest {
                path: root.join(".env"),
                max_bytes: 1024,
            },
            &root,
        );
        assert!(matches!(result, Err(FileToolError::SensitivePath(_))));
    }

    #[test]
    fn rejects_path_outside_workspace() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-ws-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let result = read_file(
            &FileReadRequest {
                path: PathBuf::from("/etc/passwd"),
                max_bytes: 1024,
            },
            &root,
        );
        assert!(matches!(
            result,
            Err(FileToolError::PathEscapesWorkspace(_))
        ));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn enhanced_sensitive_path_detection() {
        assert!(is_sensitive_path("server.key"));
        assert!(is_sensitive_path("cert.pem"));
        assert!(is_sensitive_path("keystore.pfx"));
        assert!(is_sensitive_path("config/credentials"));
        assert!(is_sensitive_path("api/token"));
        assert!(is_sensitive_path("/proc/cpuinfo"));
        assert!(is_sensitive_path("/dev/fd/3"));
    }
}
