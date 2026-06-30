use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fff::frecency::FrecencyTracker;
use fff::git::format_git_status_opt;
use fff::grep::{GrepMode, GrepSearchOptions};
use fff::query_tracker::QueryTracker;
use fff::{
    ContentCacheBudget, FFFMode, FilePicker, FilePickerOptions, FuzzySearchOptions, PaginationArgs,
    QueryParser, SharedFilePicker, SharedFrecency, SharedQueryTracker,
};
use rustyred_thg_core::RedCoreGraphStore;

use crate::{
    search_code_with_store_candidate_paths, CodeIndexError, SearchCodeInput, SearchCodeOutput,
};

const PATH_CURSOR_PREFIX: &str = "path:";
const CONTENT_CURSOR_PREFIX: &str = "content:";
const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_CONTENT_FILE_LIMIT: usize = 10;
const DEFAULT_MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct FffSearchConfig {
    pub repo_path: PathBuf,
    pub db_dir: Option<PathBuf>,
    pub enable_mmap_cache: bool,
    pub enable_content_indexing: bool,
    pub watch: bool,
    pub follow_symlinks: bool,
    pub max_cached_files: Option<usize>,
    pub ready_timeout: Duration,
}

impl FffSearchConfig {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
            db_dir: None,
            enable_mmap_cache: true,
            enable_content_indexing: true,
            watch: true,
            follow_symlinks: false,
            max_cached_files: Some(30_000),
            ready_timeout: DEFAULT_READY_TIMEOUT,
        }
    }

    pub fn with_db_dir(mut self, db_dir: impl Into<PathBuf>) -> Self {
        self.db_dir = Some(db_dir.into());
        self
    }

    pub fn without_watcher(mut self) -> Self {
        self.watch = false;
        self
    }
}

#[derive(Debug)]
pub enum FffSearchError {
    Init(String),
    NotReady { timeout: Duration },
    InvalidCursor(String),
    NotInitialized,
}

impl fmt::Display for FffSearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init(message) => write!(f, "fff initialization/search failed: {message}"),
            Self::NotReady { timeout } => write!(f, "fff index was not ready within {timeout:?}"),
            Self::InvalidCursor(cursor) => write!(f, "invalid fff cursor {cursor:?}"),
            Self::NotInitialized => write!(f, "fff file picker is not initialized"),
        }
    }
}

impl std::error::Error for FffSearchError {}

impl From<FffSearchError> for CodeIndexError {
    fn from(error: FffSearchError) -> Self {
        Self {
            code: "fff_search_error".to_string(),
            message: error.to_string(),
        }
    }
}

#[derive(Clone)]
pub struct FffRepoSearch {
    repo_path: PathBuf,
    picker: SharedFilePicker,
    frecency: SharedFrecency,
    query_tracker: SharedQueryTracker,
    ready_timeout: Duration,
    watch: bool,
}

impl FffRepoSearch {
    pub fn open(config: FffSearchConfig) -> Result<Self, FffSearchError> {
        let repo_path = canonicalize_existing(&config.repo_path)?;
        let picker = SharedFilePicker::default();
        let frecency = SharedFrecency::default();
        let query_tracker = SharedQueryTracker::default();

        if let Some(db_dir) = &config.db_dir {
            std::fs::create_dir_all(db_dir).map_err(|err| FffSearchError::Init(err.to_string()))?;
            let frecency_db = FrecencyTracker::open(db_dir.join("frecency"))
                .map_err(|err| FffSearchError::Init(err.to_string()))?;
            frecency
                .init(frecency_db)
                .map_err(|err| FffSearchError::Init(err.to_string()))?;
            let query_db = QueryTracker::open(db_dir.join("queries"))
                .map_err(|err| FffSearchError::Init(err.to_string()))?;
            query_tracker
                .init(query_db)
                .map_err(|err| FffSearchError::Init(err.to_string()))?;
        }

        FilePicker::new_with_shared_state(
            picker.clone(),
            frecency.clone(),
            FilePickerOptions {
                base_path: repo_path.to_string_lossy().into_owned(),
                enable_mmap_cache: config.enable_mmap_cache,
                enable_content_indexing: config.enable_content_indexing,
                mode: FFFMode::Ai,
                cache_budget: config
                    .max_cached_files
                    .map(ContentCacheBudget::new_for_repo),
                watch: config.watch,
                follow_symlinks: config.follow_symlinks,
                ..Default::default()
            },
        )
        .map_err(|err| FffSearchError::Init(err.to_string()))?;

        let search = Self {
            repo_path,
            picker,
            frecency,
            query_tracker,
            ready_timeout: config.ready_timeout,
            watch: config.watch,
        };
        search.wait_until_ready(config.ready_timeout)?;
        Ok(search)
    }

    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    pub fn wait_until_ready(&self, timeout: Duration) -> Result<(), FffSearchError> {
        if self.picker.wait_for_scan(timeout)
            && self.picker.wait_for_indexing_complete(timeout)
            && (!self.watch || self.picker.wait_for_watcher(timeout))
        {
            Ok(())
        } else {
            Err(FffSearchError::NotReady { timeout })
        }
    }

    pub fn refresh_git_status(&self) -> Result<usize, FffSearchError> {
        self.picker
            .refresh_git_status(&self.frecency)
            .map_err(|err| FffSearchError::Init(err.to_string()))
    }

    pub fn record_access(
        &self,
        query: Option<&str>,
        relative_path: &str,
    ) -> Result<(), FffSearchError> {
        let absolute = self.repo_path.join(relative_path);
        if let Some(tracker) = self
            .frecency
            .read()
            .map_err(|err| FffSearchError::Init(err.to_string()))?
            .as_ref()
        {
            tracker
                .track_access(&absolute)
                .map_err(|err| FffSearchError::Init(err.to_string()))?;
        }
        if let Some(query) = query {
            if let Some(tracker) = self
                .query_tracker
                .write()
                .map_err(|err| FffSearchError::Init(err.to_string()))?
                .as_mut()
            {
                tracker
                    .track_query_completion(query, &self.repo_path, Path::new(relative_path))
                    .map_err(|err| FffSearchError::Init(err.to_string()))?;
            }
        }
        let _ = self.refresh_git_status();
        Ok(())
    }

    pub fn search_paths(
        &self,
        query: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<FffPathSearchOutput, FffSearchError> {
        self.wait_until_ready(self.ready_timeout)?;
        let offset = cursor_offset(cursor, PATH_CURSOR_PREFIX)?;
        let limit = limit.max(1);

        let guard = self
            .picker
            .read()
            .map_err(|err| FffSearchError::Init(err.to_string()))?;
        let picker = guard.as_ref().ok_or(FffSearchError::NotInitialized)?;
        let query_tracker = self
            .query_tracker
            .read()
            .map_err(|err| FffSearchError::Init(err.to_string()))?;
        let parsed = QueryParser::default().parse(query);
        let result = picker.fuzzy_search(
            &parsed,
            query_tracker.as_ref(),
            FuzzySearchOptions {
                max_threads: 0,
                current_file: None,
                project_path: Some(&self.repo_path),
                combo_boost_score_multiplier: 100,
                min_combo_count: 3,
                pagination: PaginationArgs { offset, limit },
            },
        );

        let items = result
            .items
            .iter()
            .zip(result.scores.iter())
            .map(|(item, score)| FffPathHit {
                path: item.relative_path(picker),
                score: score.total,
                frecency_boost: score.frecency_boost,
                git_status_boost: score.git_status_boost,
                git_status: format_git_status_opt(item.git_status).map(str::to_string),
            })
            .collect::<Vec<_>>();
        let next_offset = offset + items.len();
        let next_cursor = (next_offset < result.total_matched)
            .then(|| format!("{PATH_CURSOR_PREFIX}{next_offset}"));

        Ok(FffPathSearchOutput {
            query: query.to_string(),
            hits: items,
            total_matched: result.total_matched,
            total_files: result.total_files,
            next_cursor,
        })
    }

    pub fn search_content(
        &self,
        query: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<FffContentSearchOutput, FffSearchError> {
        self.wait_until_ready(self.ready_timeout)?;
        let file_offset = cursor_offset(cursor, CONTENT_CURSOR_PREFIX)?;
        let limit = limit.max(1);

        let guard = self
            .picker
            .read()
            .map_err(|err| FffSearchError::Init(err.to_string()))?;
        let picker = guard.as_ref().ok_or(FffSearchError::NotInitialized)?;
        let parsed = QueryParser::default().parse(query);
        let result = picker.grep(
            &parsed,
            &GrepSearchOptions {
                max_file_size: DEFAULT_MAX_FILE_SIZE,
                max_matches_per_file: DEFAULT_CONTENT_FILE_LIMIT,
                smart_case: true,
                file_offset,
                page_limit: limit,
                mode: GrepMode::PlainText,
                time_budget_ms: 0,
                before_context: 0,
                after_context: 0,
                classify_definitions: true,
                trim_whitespace: true,
                abort_signal: None,
            },
        );

        let hits = result
            .matches
            .iter()
            .filter_map(|matched| {
                let file = result.files.get(matched.file_index)?;
                Some(FffContentHit {
                    path: file.relative_path(picker),
                    line_number: matched.line_number,
                    column: matched.col,
                    line: matched.line_content.clone(),
                    is_definition: matched.is_definition,
                    git_status: format_git_status_opt(file.git_status).map(str::to_string),
                })
            })
            .collect::<Vec<_>>();
        let next_cursor = (result.next_file_offset != 0)
            .then(|| format!("{CONTENT_CURSOR_PREFIX}{}", result.next_file_offset));

        Ok(FffContentSearchOutput {
            query: query.to_string(),
            hits,
            files_with_matches: result.files_with_matches,
            total_files: result.total_files,
            filtered_file_count: result.filtered_file_count,
            next_cursor,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FffPathHit {
    pub path: String,
    pub score: i32,
    pub frecency_boost: i32,
    pub git_status_boost: i32,
    pub git_status: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FffPathSearchOutput {
    pub query: String,
    pub hits: Vec<FffPathHit>,
    pub total_matched: usize,
    pub total_files: usize,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FffContentHit {
    pub path: String,
    pub line_number: u64,
    pub column: usize,
    pub line: String,
    pub is_definition: bool,
    pub git_status: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FffContentSearchOutput {
    pub query: String,
    pub hits: Vec<FffContentHit>,
    pub files_with_matches: usize,
    pub total_files: usize,
    pub filtered_file_count: usize,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct FffCodeSearchInput {
    pub tenant_id: String,
    pub repo_id: String,
    pub query: String,
    pub literal_limit: usize,
    pub structural_limit: u64,
}

#[derive(Clone, Debug)]
pub struct FffCodeSearchOutput {
    pub literal: FffContentSearchOutput,
    pub structural: SearchCodeOutput,
    pub candidate_paths: Vec<String>,
}

pub fn fff_then_search_code_in_store(
    store: &mut RedCoreGraphStore,
    fff: &FffRepoSearch,
    input: FffCodeSearchInput,
) -> Result<FffCodeSearchOutput, CodeIndexError> {
    let literal = fff
        .search_content(&input.query, input.literal_limit.max(1), None)
        .map_err(CodeIndexError::from)?;
    let candidate_paths = literal
        .hits
        .iter()
        .map(|hit| hit.path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let structural = search_code_with_store_candidate_paths(
        store,
        SearchCodeInput {
            tenant_id: input.tenant_id,
            query: input.query,
            repo_id: input.repo_id,
            path_prefix: String::new(),
            kinds: Vec::new(),
            limit: input.structural_limit,
        },
        Some(&candidate_paths),
    )?;
    Ok(FffCodeSearchOutput {
        literal,
        structural,
        candidate_paths,
    })
}

fn canonicalize_existing(path: &Path) -> Result<PathBuf, FffSearchError> {
    path.canonicalize()
        .map_err(|err| FffSearchError::Init(format!("canonicalize {}: {err}", path.display())))
}

fn cursor_offset(cursor: Option<&str>, prefix: &str) -> Result<usize, FffSearchError> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let raw = cursor
        .strip_prefix(prefix)
        .ok_or_else(|| FffSearchError::InvalidCursor(cursor.to_string()))?;
    raw.parse::<usize>()
        .map_err(|_| FffSearchError::InvalidCursor(cursor.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ingest_codebase_in_store, IngestCodebaseInput};
    use rustyred_thg_core::RedCoreGraphStore;

    #[test]
    fn fff_search_is_warm_paginated_gitignore_aware_and_composes_with_code_search() {
        let root = unique_temp_dir("fff-search");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "pub struct SearchKernel {}\n\npub fn run_search() -> usize { 1 }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/helper.rs"),
            "pub fn helper() -> usize { 2 }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("target/ignored.rs"),
            "pub fn ignored_search() -> usize { 0 }\n",
        )
        .unwrap();
        git(&root, &["init", "-b", "main"]);
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "seed", "--no-gpg-sign"]);
        std::fs::write(
            root.join("src/lib.rs"),
            "pub struct SearchKernel {}\n\npub fn run_search() -> usize { 1 }\n// dirty\n",
        )
        .unwrap();

        let fff = FffRepoSearch::open(
            FffSearchConfig::new(&root)
                .with_db_dir(root.join(".fff-db"))
                .without_watcher(),
        )
        .unwrap();
        fff.refresh_git_status().unwrap();
        fff.record_access(Some("helper"), "src/helper.rs").unwrap();

        let first = fff.search_paths("rs", 1, None).unwrap();
        assert_eq!(first.hits.len(), 1);
        assert!(first.next_cursor.is_some(), "first page returns a cursor");
        assert!(
            !first.hits.iter().any(|hit| hit.path.contains("target/")),
            "gitignored paths stay out of the warm index"
        );
        let second = fff
            .search_paths("rs", 5, first.next_cursor.as_deref())
            .unwrap();
        assert!(
            second.hits.iter().all(|hit| !hit.path.contains("target/")),
            "follow-on page uses the fff cursor offset without reintroducing ignored files"
        );

        let content = fff.search_content("run_search", 10, None).unwrap();
        assert_eq!(content.hits[0].path, "src/lib.rs");
        assert!(content.hits[0].is_definition);

        let mut store = RedCoreGraphStore::memory();
        ingest_codebase_in_store(
            &mut store,
            IngestCodebaseInput {
                tenant_id: "tenant".to_string(),
                repo_path: root.to_string_lossy().into_owned(),
                repo_id: "repo".to_string(),
                include_extensions: vec!["rs".to_string()],
                exclude_dirs: vec![".git".to_string(), ".fff-db".to_string()],
                max_files: 100,
                max_file_bytes: 1024 * 1024,
                max_total_bytes: 5 * 1024 * 1024,
                materialize_symbol_name_index: false,
                actor: "test".to_string(),
            },
        )
        .unwrap();
        let combined = fff_then_search_code_in_store(
            &mut store,
            &fff,
            FffCodeSearchInput {
                tenant_id: "tenant".to_string(),
                repo_id: "repo".to_string(),
                query: "run_search".to_string(),
                literal_limit: 10,
                structural_limit: 10,
            },
        )
        .unwrap();
        assert_eq!(combined.candidate_paths, vec!["src/lib.rs".to_string()]);
        assert_eq!(combined.structural.hits[0].file_path, "src/lib.rs");
        assert_eq!(combined.structural.hits[0].name, "run_search");

        let _ = std::fs::remove_dir_all(root);
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "theorem-{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git(cwd: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "Theorem Test")
            .env("GIT_AUTHOR_EMAIL", "theorem@example.test")
            .env("GIT_COMMITTER_NAME", "Theorem Test")
            .env("GIT_COMMITTER_EMAIL", "theorem@example.test")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
