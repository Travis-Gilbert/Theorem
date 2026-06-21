use async_trait::async_trait;
use rustyred_web::{
    run_action, ActionOptions, ActuationPlan, ActuationReceipt, AutomationActionReceipt,
    BrowserActionPolicy, BrowserDriver, BrowserEngineError, BrowserEngineResult, ElementBox,
    InteractiveElement, Locator, LocatorAction, PageState,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use theorem_browser_agent::{VisualPerceiverElement, VisualPerceiverResponse};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::Mutex;
#[cfg(test)]
use theorem_browser_agent::{
    VisualPerceiverBox, VisualPerceiverBoxPixels, VisualPerceiverImageSize,
};

const LIVE_BROWSER_ENDPOINT_ENV: &str = "THEOREM_LIVE_BROWSER_ENDPOINT";
/// Pool size bound (handoff-7 D1: "bounded by a pool size"). The pool vends at most
/// this many concurrent live sessions; each checkout holds a permit until the session
/// drops (returns on completion). Override with `THEOREM_LIVE_BROWSER_POOL_SIZE`.
const LIVE_BROWSER_POOL_SIZE_ENV: &str = "THEOREM_LIVE_BROWSER_POOL_SIZE";
const DEFAULT_LIVE_BROWSER_POOL_SIZE: usize = 4;
const VISUAL_PERCEIVER_ENDPOINT_ENV: &str = "THEOREM_VISUAL_PERCEIVER_URL";
const VISUAL_PERCEIVER_ENDPOINT_ALIAS_ENV: &str = "THEOREM_OMNIPARSER_URL";

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
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_screenshot: bool,
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
    visual_perceiver: Option<Arc<VisualPerceiverClient>>,
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
            visual_perceiver: VisualPerceiverClient::from_env().map(Arc::new),
            semaphore: Arc::new(Semaphore::new(capacity)),
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    async fn augment_page_with_visual_perception(
        &self,
        page: &mut PageState,
        screenshot_base64: Option<&str>,
        screenshot_media_type: Option<&str>,
    ) {
        if !page.interactive_elements.is_empty() {
            return;
        }
        let Some(client) = &self.visual_perceiver else {
            return;
        };
        let Some(image_base64) = screenshot_base64 else {
            stamp_visual_perceiver_fetch(
                page,
                json!({
                    "status": "missing_screenshot",
                    "message": "visual perceiver configured, but the browser sidecar did not return a screenshot"
                }),
            );
            return;
        };
        match client
            .parse(image_base64, screenshot_media_type.unwrap_or("image/png"))
            .await
        {
            Ok(response) => merge_visual_elements(page, &response),
            Err(error) => stamp_visual_perceiver_fetch(
                page,
                json!({
                    "status": "error",
                    "message": error
                }),
            ),
        }
    }
}

#[derive(Clone)]
struct VisualPerceiverClient {
    client: reqwest::Client,
    endpoint: String,
}

impl VisualPerceiverClient {
    fn from_env() -> Option<Self> {
        let endpoint = std::env::var(VISUAL_PERCEIVER_ENDPOINT_ENV)
            .or_else(|_| std::env::var(VISUAL_PERCEIVER_ENDPOINT_ALIAS_ENV))
            .ok()?;
        let endpoint = endpoint.trim().trim_end_matches('/').to_string();
        if endpoint.is_empty() {
            return None;
        }
        let endpoint = if endpoint.ends_with("/parse") {
            endpoint
        } else {
            format!("{endpoint}/parse")
        };
        Some(Self {
            client: reqwest::Client::new(),
            endpoint,
        })
    }

    async fn parse(
        &self,
        image_base64: &str,
        media_type: &str,
    ) -> Result<VisualPerceiverResponse, String> {
        let response = self
            .client
            .post(&self.endpoint)
            .json(&json!({
                "image_base64": image_base64,
                "media_type": media_type,
                "use_ocr": true,
                "caption": false,
                "include_annotated": false
            }))
            .send()
            .await
            .map_err(|error| format!("visual perceiver request failed: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!(
                "visual perceiver returned {status}: {}",
                body.chars().take(240).collect::<String>()
            ));
        }
        response
            .json::<VisualPerceiverResponse>()
            .await
            .map_err(|error| format!("visual perceiver response was not parseable: {error}"))
    }
}

fn merge_visual_elements(page: &mut PageState, response: &VisualPerceiverResponse) {
    let start_count = page.interactive_elements.len();
    page.interactive_elements.extend(
        response
            .elements
            .iter()
            .map(visual_element_to_interactive_element),
    );
    stamp_visual_perceiver_fetch(
        page,
        json!({
            "status": "parsed",
            "count": response.count,
            "added_elements": page.interactive_elements.len().saturating_sub(start_count),
            "image_size": response.image_size
        }),
    );
}

fn visual_element_to_interactive_element(element: &VisualPerceiverElement) -> InteractiveElement {
    InteractiveElement {
        element_id: element.element_id(),
        role: element.role(),
        name: element.label(),
        value: None,
        test_id: Some("visual-perceiver".to_string()),
        bbox: element.box_pixels.as_ref().map(|bbox| ElementBox {
            x: bbox.x1,
            y: bbox.y1,
            width: (bbox.x2 - bbox.x1).max(0),
            height: (bbox.y2 - bbox.y1).max(0),
        }),
        visible: true,
        enabled: element.interactable,
        editable: false,
        degraded: false,
    }
}

fn stamp_visual_perceiver_fetch(page: &mut PageState, value: Value) {
    match &mut page.fetch {
        Some(Value::Object(map)) => {
            map.insert("visual_perceiver".to_string(), value);
        }
        _ => {
            page.fetch = Some(json!({ "visual_perceiver": value }));
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Deserialize)]
struct RemoteCheckoutResponse {
    session_id: String,
    page: PageState,
    #[serde(default)]
    screenshot_base64: Option<String>,
    #[serde(default)]
    screenshot_media_type: Option<String>,
}

#[derive(Debug, Serialize)]
struct RemoteSnapshotRequest {
    session_id: String,
    #[serde(default, skip_serializing_if = "is_false")]
    include_screenshot: bool,
}

#[derive(Debug, Deserialize)]
struct RemoteSnapshotResponse {
    page: PageState,
    #[serde(default)]
    screenshot_base64: Option<String>,
    #[serde(default)]
    screenshot_media_type: Option<String>,
}

#[derive(Debug, Serialize)]
struct RemoteActuateRequest {
    session_id: String,
    plan: ActuationPlan,
    policy: BrowserActionPolicy,
    #[serde(default, skip_serializing_if = "is_false")]
    include_screenshot: bool,
}

#[derive(Debug, Deserialize)]
struct RemoteActuateResponse {
    receipt: ActuationReceipt,
    #[serde(default)]
    page: Option<PageState>,
    #[serde(default)]
    screenshot_base64: Option<String>,
    #[serde(default)]
    screenshot_media_type: Option<String>,
}

#[async_trait]
impl LiveBrowserPool for RemoteBrowserPool {
    async fn checkout(
        &self,
        mut request: BrowserCheckoutRequest,
    ) -> BrowserEngineResult<Box<dyn LiveBrowserSession>> {
        // Acquire a pool slot first (D1 bound). Awaits when the pool is saturated;
        // the permit rides on the session and frees the slot on drop.
        let permit = self.semaphore.clone().acquire_owned().await.map_err(|_| {
            BrowserEngineError::Backend {
                message: "live browser pool is closed".to_string(),
            }
        })?;
        request.include_screenshot = request.include_screenshot || self.visual_perceiver.is_some();
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
        let mut page = checkout.page;
        self.augment_page_with_visual_perception(
            &mut page,
            checkout.screenshot_base64.as_deref(),
            checkout.screenshot_media_type.as_deref(),
        )
        .await;
        Ok(Box::new(RemoteBrowserSession {
            pool: self.clone(),
            session_id: checkout.session_id,
            current_page: page,
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
                include_screenshot: self.pool.visual_perceiver.is_some(),
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
        let mut page = snapshot.page;
        self.pool
            .augment_page_with_visual_perception(
                &mut page,
                snapshot.screenshot_base64.as_deref(),
                snapshot.screenshot_media_type.as_deref(),
            )
            .await;
        self.current_page = page;
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
                include_screenshot: self.pool.visual_perceiver.is_some(),
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
        if let Some(mut page) = actuation.page.clone() {
            self.pool
                .augment_page_with_visual_perception(
                    &mut page,
                    actuation.screenshot_base64.as_deref(),
                    actuation.screenshot_media_type.as_deref(),
                )
                .await;
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
            visual_perceiver: None,
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

    #[test]
    fn visual_perceiver_response_adds_visual_interactive_elements() {
        let mut page = PageState {
            url: "app://canvas".to_string(),
            title: "Canvas".to_string(),
            distilled_text: String::new(),
            interactive_elements: Vec::new(),
            active_tab_id: None,
            fetch: None,
        };
        merge_visual_elements(
            &mut page,
            &VisualPerceiverResponse {
                image_size: VisualPerceiverImageSize {
                    width: 320,
                    height: 200,
                },
                count: 1,
                elements: vec![VisualPerceiverElement {
                    id: 3,
                    interactable: true,
                    source: "icon".to_string(),
                    content: "Submit".to_string(),
                    score: 0.88,
                    box_pixels: Some(VisualPerceiverBoxPixels {
                        x1: 12,
                        y1: 24,
                        x2: 132,
                        y2: 64,
                    }),
                    normalized_box: VisualPerceiverBox {
                        x: 0.04,
                        y: 0.12,
                        w: 0.38,
                        h: 0.2,
                    },
                }],
                annotated_image_base64: None,
                annotated_media_type: None,
            },
        );

        assert_eq!(page.interactive_elements.len(), 1);
        let element = &page.interactive_elements[0];
        assert_eq!(element.element_id, "visual:3");
        assert_eq!(element.role, "button");
        assert_eq!(element.name, "Submit");
        assert_eq!(
            element.bbox,
            Some(ElementBox {
                x: 12,
                y: 24,
                width: 120,
                height: 40
            })
        );
        assert_eq!(
            page.fetch.as_ref().unwrap()["visual_perceiver"]["status"],
            "parsed"
        );
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
