use async_trait::async_trait;
use rustyred_web::{
    run_action, ActionOptions, ActuationPlan, ActuationReceipt, AutomationActionReceipt,
    BrowserActionPolicy, BrowserDriver, BrowserEngineError, BrowserEngineResult, Locator,
    LocatorAction, PageState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[cfg(test)]
use rustyred_web::{ElementBox, InteractiveElement};
#[cfg(test)]
use serde_json::json;
#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::Mutex;

const LIVE_BROWSER_ENDPOINT_ENV: &str = "THEOREM_LIVE_BROWSER_ENDPOINT";
/// Pool size bound (handoff-7 D1: "bounded by a pool size"). The pool vends at most
/// this many concurrent live sessions; each checkout holds a permit until the session
/// drops (returns on completion). Override with `THEOREM_LIVE_BROWSER_POOL_SIZE`.
const LIVE_BROWSER_POOL_SIZE_ENV: &str = "THEOREM_LIVE_BROWSER_POOL_SIZE";
const DEFAULT_LIVE_BROWSER_POOL_SIZE: usize = 4;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserCheckoutRequest {
    pub tenant: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub max_bytes: usize,
    pub actor_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserActionCommand {
    pub locator: Locator,
    pub action: LocatorAction,
    #[serde(default)]
    pub options: ActionOptions,
}

#[derive(Clone, Debug, Default)]
pub struct BrowserLiveSessionRecord {
    pub run_id: String,
    pub session_id: String,
    pub pending_action: Option<Value>,
    pub last_page: Option<PageState>,
    pub demonstrations: Vec<Value>,
}

impl BrowserLiveSessionRecord {
    pub fn new(run_id: String, session_id: String) -> Self {
        Self {
            run_id,
            session_id,
            pending_action: None,
            last_page: None,
            demonstrations: Vec::new(),
        }
    }
}

#[async_trait]
pub trait LiveBrowserPool: Send + Sync {
    async fn checkout(
        &self,
        request: BrowserCheckoutRequest,
    ) -> BrowserEngineResult<Box<dyn LiveBrowserSession>>;

    fn transport(&self) -> &'static str;
}

#[async_trait]
pub trait LiveBrowserSession: Send {
    fn session_id(&self) -> &str;
    fn current_page(&self) -> BrowserEngineResult<PageState>;

    async fn run_action(
        &mut self,
        command: &BrowserActionCommand,
        policy: &BrowserActionPolicy,
    ) -> BrowserEngineResult<AutomationActionReceipt>;
}

#[derive(Clone)]
pub struct RemoteBrowserPool {
    client: reqwest::Client,
    base_url: String,
    // The pool-size bound (D1). Cloned with the pool; a checkout acquires an owned
    // permit that rides on the session and frees the slot on drop. The semaphore's
    // permit count IS the bound; the max comes from the env at construction.
    semaphore: Arc<Semaphore>,
}

impl RemoteBrowserPool {
    pub fn from_env() -> Option<Self> {
        let endpoint = std::env::var(LIVE_BROWSER_ENDPOINT_ENV).ok()?;
        let endpoint = endpoint.trim().trim_end_matches('/').to_string();
        if endpoint.is_empty() {
            return None;
        }
        let capacity = std::env::var(LIVE_BROWSER_POOL_SIZE_ENV)
            .ok()
            .and_then(|raw| raw.trim().parse::<usize>().ok())
            .filter(|size| *size > 0)
            .unwrap_or(DEFAULT_LIVE_BROWSER_POOL_SIZE);
        Some(Self {
            client: reqwest::Client::new(),
            base_url: endpoint,
            semaphore: Arc::new(Semaphore::new(capacity)),
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }
}

#[derive(Debug, Deserialize)]
struct RemoteCheckoutResponse {
    session_id: String,
    page: PageState,
}

#[derive(Debug, Serialize)]
struct RemoteSnapshotRequest {
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct RemoteSnapshotResponse {
    page: PageState,
}

#[derive(Debug, Serialize)]
struct RemoteActuateRequest {
    session_id: String,
    plan: ActuationPlan,
    policy: BrowserActionPolicy,
}

#[derive(Debug, Deserialize)]
struct RemoteActuateResponse {
    receipt: ActuationReceipt,
    #[serde(default)]
    page: Option<PageState>,
}

#[async_trait]
impl LiveBrowserPool for RemoteBrowserPool {
    async fn checkout(
        &self,
        request: BrowserCheckoutRequest,
    ) -> BrowserEngineResult<Box<dyn LiveBrowserSession>> {
        // Acquire a pool slot first (D1 bound). Awaits when the pool is saturated;
        // the permit rides on the session and frees the slot on drop.
        let permit = self.semaphore.clone().acquire_owned().await.map_err(|_| {
            BrowserEngineError::Backend {
                message: "live browser pool is closed".to_string(),
            }
        })?;
        let response = self
            .client
            .post(self.endpoint("sessions/checkout"))
            .json(&request)
            .send()
            .await
            .map_err(remote_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(BrowserEngineError::Backend {
                message: format!("live browser checkout returned {status}"),
            });
        }
        let checkout = response
            .json::<RemoteCheckoutResponse>()
            .await
            .map_err(remote_error)?;
        Ok(Box::new(RemoteBrowserSession {
            pool: self.clone(),
            session_id: checkout.session_id,
            current_page: checkout.page,
            _permit: permit,
        }))
    }

    fn transport(&self) -> &'static str {
        "remote_sidecar"
    }
}

struct RemoteBrowserSession {
    pool: RemoteBrowserPool,
    session_id: String,
    current_page: PageState,
    // Held for the session's lifetime; dropping it returns the slot to the pool.
    _permit: OwnedSemaphorePermit,
}

impl RemoteBrowserSession {
    async fn refresh_snapshot(&mut self) -> BrowserEngineResult<()> {
        let response = self
            .pool
            .client
            .post(self.pool.endpoint("sessions/snapshot"))
            .json(&RemoteSnapshotRequest {
                session_id: self.session_id.clone(),
            })
            .send()
            .await
            .map_err(remote_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(BrowserEngineError::Backend {
                message: format!("live browser snapshot returned {status}"),
            });
        }
        let snapshot = response
            .json::<RemoteSnapshotResponse>()
            .await
            .map_err(remote_error)?;
        self.current_page = snapshot.page;
        Ok(())
    }
}

impl BrowserDriver for RemoteBrowserSession {
    fn snapshot(&self) -> BrowserEngineResult<PageState> {
        Ok(self.current_page.clone())
    }

    async fn actuate(
        &mut self,
        plan: ActuationPlan,
        policy: &BrowserActionPolicy,
    ) -> BrowserEngineResult<ActuationReceipt> {
        let response = self
            .pool
            .client
            .post(self.pool.endpoint("sessions/actuate"))
            .json(&RemoteActuateRequest {
                session_id: self.session_id.clone(),
                plan,
                policy: policy.clone(),
            })
            .send()
            .await
            .map_err(remote_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(BrowserEngineError::Backend {
                message: format!("live browser actuation returned {status}"),
            });
        }
        let actuation = response
            .json::<RemoteActuateResponse>()
            .await
            .map_err(remote_error)?;
        if let Some(page) = actuation.page.clone() {
            self.current_page = page;
        } else {
            self.refresh_snapshot().await?;
        }
        Ok(actuation.receipt)
    }
}

#[async_trait]
impl LiveBrowserSession for RemoteBrowserSession {
    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn current_page(&self) -> BrowserEngineResult<PageState> {
        Ok(self.current_page.clone())
    }

    async fn run_action(
        &mut self,
        command: &BrowserActionCommand,
        policy: &BrowserActionPolicy,
    ) -> BrowserEngineResult<AutomationActionReceipt> {
        self.refresh_snapshot().await?;
        run_action(
            self,
            &command.locator,
            command.action.clone(),
            command.options.clone(),
            policy,
        )
        .await
    }
}

fn remote_error(error: reqwest::Error) -> BrowserEngineError {
    BrowserEngineError::Backend {
        message: format!("live browser sidecar request failed: {error}"),
    }
}

#[cfg(test)]
#[derive(Clone)]
pub struct ScriptedBrowserPool {
    sessions: Arc<Mutex<BTreeMap<String, PageState>>>,
    receipts: Arc<Mutex<Vec<AutomationActionReceipt>>>,
    initial_page: PageState,
}

#[cfg(test)]
impl ScriptedBrowserPool {
    pub fn new(initial_page: PageState) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(BTreeMap::new())),
            receipts: Arc::new(Mutex::new(Vec::new())),
            initial_page,
        }
    }

    pub fn receipts(&self) -> Vec<AutomationActionReceipt> {
        self.receipts
            .lock()
            .map(|receipts| receipts.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
#[async_trait]
impl LiveBrowserPool for ScriptedBrowserPool {
    async fn checkout(
        &self,
        request: BrowserCheckoutRequest,
    ) -> BrowserEngineResult<Box<dyn LiveBrowserSession>> {
        let session_id = request
            .session_id
            .unwrap_or_else(|| format!("session:{}", request.run_id));
        let page = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| BrowserEngineError::Backend {
                    message: "scripted browser session lock poisoned".to_string(),
                })?;
            sessions
                .entry(session_id.clone())
                .or_insert_with(|| self.initial_page.clone())
                .clone()
        };
        Ok(Box::new(ScriptedBrowserSession {
            session_id,
            current_page: page,
            sessions: self.sessions.clone(),
            receipts: self.receipts.clone(),
        }))
    }

    fn transport(&self) -> &'static str {
        "scripted_test"
    }
}

#[cfg(test)]
struct ScriptedBrowserSession {
    session_id: String,
    current_page: PageState,
    sessions: Arc<Mutex<BTreeMap<String, PageState>>>,
    receipts: Arc<Mutex<Vec<AutomationActionReceipt>>>,
}

#[cfg(test)]
impl BrowserDriver for ScriptedBrowserSession {
    fn snapshot(&self) -> BrowserEngineResult<PageState> {
        Ok(self.current_page.clone())
    }

    async fn actuate(
        &mut self,
        plan: ActuationPlan,
        _policy: &BrowserActionPolicy,
    ) -> BrowserEngineResult<ActuationReceipt> {
        apply_scripted_actuation(&mut self.current_page, &plan)?;
        self.sessions
            .lock()
            .map_err(|_| BrowserEngineError::Backend {
                message: "scripted browser session lock poisoned".to_string(),
            })?
            .insert(self.session_id.clone(), self.current_page.clone());
        Ok(ActuationReceipt {
            mechanism: "scripted_test_driver".to_string(),
            detail: json!({
                "target": plan.target_handle,
                "kind": plan.kind,
            }),
        })
    }
}

#[cfg(test)]
#[async_trait]
impl LiveBrowserSession for ScriptedBrowserSession {
    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn current_page(&self) -> BrowserEngineResult<PageState> {
        Ok(self.current_page.clone())
    }

    async fn run_action(
        &mut self,
        command: &BrowserActionCommand,
        policy: &BrowserActionPolicy,
    ) -> BrowserEngineResult<AutomationActionReceipt> {
        let receipt = run_action(
            self,
            &command.locator,
            command.action.clone(),
            command.options.clone(),
            policy,
        )
        .await?;
        self.receipts
            .lock()
            .map_err(|_| BrowserEngineError::Backend {
                message: "scripted browser receipt lock poisoned".to_string(),
            })?
            .push(receipt.clone());
        Ok(receipt)
    }
}

#[cfg(test)]
fn apply_scripted_actuation(page: &mut PageState, plan: &ActuationPlan) -> BrowserEngineResult<()> {
    let element = page
        .interactive_elements
        .iter_mut()
        .find(|element| element.element_id == plan.target_handle)
        .ok_or_else(|| BrowserEngineError::ElementNotFound {
            element_id: plan.target_handle.clone(),
        })?;
    match &plan.kind {
        rustyred_web::ActuationKind::Keyboard { text, .. } => {
            element.value = Some(text.clone());
            page.distilled_text = format!("{} {}", page.distilled_text, text)
                .trim()
                .to_string();
        }
        rustyred_web::ActuationKind::CoordinateSynthesis { .. } => {
            element.value = Some("clicked".to_string());
        }
        rustyred_web::ActuationKind::Scroll { .. }
        | rustyred_web::ActuationKind::EmbedderControl { .. } => {}
        rustyred_web::ActuationKind::SemanticActivation { .. } => {
            return Err(BrowserEngineError::UnsupportedAction {
                reason: "scripted test driver does not implement semantic activation".to_string(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
pub fn scripted_page_with_textbox() -> PageState {
    PageState {
        url: "https://example.test/form".to_string(),
        title: "Example Form".to_string(),
        distilled_text: "Example Form".to_string(),
        interactive_elements: vec![InteractiveElement {
            element_id: "name".to_string(),
            role: "textbox".to_string(),
            name: "Name".to_string(),
            value: None,
            test_id: Some("name-input".to_string()),
            bbox: Some(ElementBox {
                x: 10,
                y: 20,
                width: 100,
                height: 20,
            }),
            visible: true,
            enabled: true,
            editable: true,
            degraded: false,
        }],
        active_tab_id: Some("tab-1".to_string()),
        fetch: None,
    }
}

#[cfg(test)]
impl RemoteBrowserPool {
    /// Construct a bounded pool without reading global env (race-free for tests).
    pub(crate) fn for_test(base_url: &str, capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            semaphore: Arc::new(Semaphore::new(capacity)),
        }
    }
}

#[cfg(test)]
mod pool_bound_tests {
    use super::*;

    #[test]
    fn the_pool_starts_at_full_capacity() {
        let pool = RemoteBrowserPool::for_test("http://localhost:9", 3);
        assert_eq!(pool.semaphore.available_permits(), 3);
    }

    #[tokio::test]
    async fn an_acquired_slot_is_returned_on_drop() {
        // Mirrors checkout's acquire-before-hop: a taken slot bounds concurrency,
        // and dropping the permit (as the session does on drop) frees it.
        let pool = RemoteBrowserPool::for_test("http://localhost:9", 1);
        let permit = pool
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("permit");
        assert_eq!(
            pool.semaphore.available_permits(),
            0,
            "the only slot is taken while checked out"
        );
        drop(permit);
        assert_eq!(
            pool.semaphore.available_permits(),
            1,
            "the slot returns to the pool on drop"
        );
    }
}
