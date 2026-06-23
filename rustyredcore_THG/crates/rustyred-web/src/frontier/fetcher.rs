use async_trait::async_trait;

use crate::{
    extract_links_for_url, global_robots_cache, FetchCascade, FetchCascadeOptions, RustyWebError,
};

use super::model::{DiscoveredLink, FetchOutcome, FetchTask};

#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn fetch(&self, task: FetchTask) -> FetchOutcome;
}

#[derive(Clone, Debug)]
pub struct CascadeFetcher {
    cascade: FetchCascade,
    max_bytes: usize,
    respect_robots: bool,
}

impl CascadeFetcher {
    pub fn new(cascade: FetchCascade, max_bytes: usize) -> Self {
        Self {
            cascade,
            max_bytes,
            respect_robots: true,
        }
    }

    pub fn from_options(
        options: FetchCascadeOptions,
        max_bytes: usize,
    ) -> Result<Self, RustyWebError> {
        Ok(Self::new(FetchCascade::new(options)?, max_bytes))
    }

    pub fn with_respect_robots(mut self, respect_robots: bool) -> Self {
        self.respect_robots = respect_robots;
        self
    }
}

#[async_trait]
impl Fetcher for CascadeFetcher {
    async fn fetch(&self, task: FetchTask) -> FetchOutcome {
        if self.respect_robots {
            match global_robots_cache()
                .check(self.cascade.client(), &task.url, self.cascade.user_agent())
                .await
            {
                Ok(decision) if !decision.allowed => {
                    return FetchOutcome::Skipped {
                        reason: format!("robots disallowed: {}", decision.reason),
                    };
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = error;
                    return FetchOutcome::Error {
                        status: None,
                        retryable: true,
                    };
                }
            }
        }

        let result = match self
            .cascade
            .fetch_with_promotion(&task.url, self.max_bytes)
            .await
        {
            Ok(result) => result,
            Err(_) => {
                return FetchOutcome::Error {
                    status: None,
                    retryable: true,
                };
            }
        };
        if result.truncated {
            return FetchOutcome::Error {
                status: Some(result.http_status),
                retryable: false,
            };
        }

        let html = String::from_utf8_lossy(&result.html_bytes);
        let links = if (200..400).contains(&result.http_status)
            && result.content_type.to_ascii_lowercase().contains("html")
        {
            extract_links_for_url(&result.final_url, &html)
                .unwrap_or_default()
                .into_iter()
                .map(|url_raw| DiscoveredLink {
                    url_raw,
                    anchor_text: String::new(),
                    rel: String::new(),
                })
                .collect()
        } else {
            Vec::new()
        };

        FetchOutcome::Ok {
            final_url: result.final_url,
            status: result.http_status,
            content_hash: *blake3::hash(&result.html_bytes).as_bytes(),
            etag: (!result.etag.is_empty()).then_some(result.etag),
            links,
        }
    }
}

#[cfg(feature = "servo")]
#[derive(Clone, Debug)]
pub struct ServoFetcher {
    inner: CascadeFetcher,
}

#[cfg(feature = "servo")]
impl ServoFetcher {
    pub fn new(inner: CascadeFetcher) -> Self {
        Self { inner }
    }
}

#[cfg(feature = "servo")]
#[async_trait]
impl Fetcher for ServoFetcher {
    async fn fetch(&self, task: FetchTask) -> FetchOutcome {
        self.inner.fetch(task).await
    }
}

#[cfg(feature = "spider_fetch")]
#[derive(Clone, Debug, Default)]
pub struct SpiderFetcher;

#[cfg(feature = "spider_fetch")]
#[async_trait]
impl Fetcher for SpiderFetcher {
    async fn fetch(&self, task: FetchTask) -> FetchOutcome {
        use spider::traits::PageData;
        use spider::website::Website;

        let mut website = Website::new(&task.url);
        website.with_limit(1);
        website.with_depth(1);
        website.scrape().await;
        let Some(page) = website.get_pages().and_then(|pages| pages.first()) else {
            return FetchOutcome::Error {
                status: None,
                retryable: true,
            };
        };
        let html = page.html();
        let final_url = page.url_final().to_string();
        let links = extract_links_for_url(&final_url, &html)
            .unwrap_or_default()
            .into_iter()
            .map(|url_raw| DiscoveredLink {
                url_raw,
                anchor_text: String::new(),
                rel: String::new(),
            })
            .collect();
        FetchOutcome::Ok {
            final_url,
            status: page.status_code().as_u16(),
            content_hash: *blake3::hash(html.as_bytes()).as_bytes(),
            etag: page
                .headers()
                .and_then(|headers| headers.get(reqwest::header::ETAG))
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
            links,
        }
    }
}
