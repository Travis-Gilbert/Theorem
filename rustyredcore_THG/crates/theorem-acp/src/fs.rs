use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AcpError, AcpResult};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReadTextFileRequest {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WriteTextFileRequest {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FileWriteReview {
    pub request_id: String,
    pub path: String,
    pub content: String,
    pub previous_content: Option<String>,
}

#[derive(Debug)]
pub struct WorkspaceFs {
    root: PathBuf,
    pending_writes: BTreeMap<String, FileWriteReview>,
}

impl WorkspaceFs {
    pub fn new(root: impl Into<PathBuf>) -> AcpResult<Self> {
        let root = root.into().canonicalize()?;
        Ok(Self {
            root,
            pending_writes: BTreeMap::new(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn read_text_file(&self, request: ReadTextFileRequest) -> AcpResult<String> {
        let path = self.resolve_existing(&request.path)?;
        let content = fs::read_to_string(path)?;
        Ok(slice_lines(&content, request.line, request.limit))
    }

    pub fn stage_write_text_file(
        &mut self,
        request: WriteTextFileRequest,
    ) -> AcpResult<FileWriteReview> {
        let path = self.resolve_write_target(&request.path)?;
        let previous_content = if path.exists() {
            Some(fs::read_to_string(&path)?)
        } else {
            None
        };
        let review = FileWriteReview {
            request_id: format!("file-write-{}", Uuid::new_v4()),
            path: path.to_string_lossy().into_owned(),
            content: request.content,
            previous_content,
        };
        self.pending_writes
            .insert(review.request_id.clone(), review.clone());
        Ok(review)
    }

    pub fn approve_write(&mut self, request_id: &str) -> AcpResult<FileWriteReview> {
        let review = self
            .pending_writes
            .remove(request_id)
            .ok_or_else(|| AcpError::PendingWriteNotFound(request_id.to_string()))?;
        let path = PathBuf::from(&review.path);
        fs::write(path, &review.content)?;
        Ok(review)
    }

    pub fn deny_write(&mut self, request_id: &str) -> AcpResult<FileWriteReview> {
        self.pending_writes
            .remove(request_id)
            .ok_or_else(|| AcpError::PendingWriteNotFound(request_id.to_string()))
    }

    pub fn pending_write(&self, request_id: &str) -> Option<&FileWriteReview> {
        self.pending_writes.get(request_id)
    }

    fn resolve_existing(&self, requested: &str) -> AcpResult<PathBuf> {
        let path = self.absolute_path(requested).canonicalize()?;
        self.ensure_in_workspace(path)
    }

    fn resolve_write_target(&self, requested: &str) -> AcpResult<PathBuf> {
        let path = self.absolute_path(requested);
        let parent = path
            .parent()
            .ok_or_else(|| AcpError::MissingParent(path.clone()))?;
        let parent = parent
            .canonicalize()
            .map_err(|_| AcpError::MissingParent(path.clone()))?;
        self.ensure_in_workspace(parent)?;
        Ok(path)
    }

    fn absolute_path(&self, requested: &str) -> PathBuf {
        let path = PathBuf::from(requested);
        if path.is_absolute() {
            path
        } else {
            self.root.join(path)
        }
    }

    fn ensure_in_workspace(&self, path: PathBuf) -> AcpResult<PathBuf> {
        if path.starts_with(&self.root) {
            Ok(path)
        } else {
            Err(AcpError::OutsideWorkspace(path))
        }
    }
}

fn slice_lines(content: &str, line: Option<usize>, limit: Option<usize>) -> String {
    let Some(line) = line else {
        return content.to_string();
    };
    let start = line.saturating_sub(1);
    let limit = limit.unwrap_or(usize::MAX);
    content
        .lines()
        .skip(start)
        .take(limit)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_are_scoped_to_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.md");
        fs::write(&file, "one\ntwo\nthree").unwrap();

        let fs = WorkspaceFs::new(dir.path()).unwrap();
        let content = fs
            .read_text_file(ReadTextFileRequest {
                path: "note.md".to_string(),
                line: Some(2),
                limit: Some(1),
            })
            .unwrap();

        assert_eq!(content, "two");
    }

    #[test]
    fn write_is_reviewed_before_commit() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.md");
        fs::write(&file, "old").unwrap();

        let mut fs = WorkspaceFs::new(dir.path()).unwrap();
        let review = fs
            .stage_write_text_file(WriteTextFileRequest {
                path: "note.md".to_string(),
                content: "new".to_string(),
            })
            .unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "old");
        assert_eq!(review.previous_content.as_deref(), Some("old"));

        fs.approve_write(&review.request_id).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
    }

    #[test]
    fn outside_workspace_paths_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let fs = WorkspaceFs::new(dir.path()).unwrap();

        let err = fs
            .read_text_file(ReadTextFileRequest {
                path: outside.path().to_string_lossy().into_owned(),
                line: None,
                limit: None,
            })
            .unwrap_err();

        assert!(matches!(err, AcpError::OutsideWorkspace(_)));
    }
}
