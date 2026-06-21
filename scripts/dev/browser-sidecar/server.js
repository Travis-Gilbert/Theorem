// Minimal Playwright sidecar for the two Theorem browser surfaces.
//
//   Surface 1 (rendered fetch):  POST /render
//       body  { action: "navigate_render_extract", url, max_bytes }
//       reply { html, http_status, content_type, final_url }
//
//   Surface 2 (live action loop): POST /sessions/checkout|snapshot|actuate
//       checkout body  { tenant, run_id, session_id?, url?, max_bytes, actor_id, include_screenshot? }
//                reply { session_id, page: PageState, screenshot_base64?, screenshot_media_type? }
//       snapshot body  { session_id, include_screenshot? }
//                reply { page: PageState, screenshot_base64?, screenshot_media_type? }
//       actuate  body  { session_id, plan: { target_handle, kind }, policy, include_screenshot? }
//                reply { receipt: { mechanism, detail }, page: PageState, screenshot_base64?, screenshot_media_type? }
//
// PageState is produced by running pilot-core's exact GEOMETRY_SNAPSHOT_SCRIPT
// in the page, so the Rust locator/actionability layer sees an identical shape.

import express from "express";
import { chromium } from "playwright";

const PORT = Number(process.env.PORT || 9223);
const DISTILLED_MAX = Number(process.env.SIDECAR_DISTILLED_TEXT_MAX || 20000);
const NAV_TIMEOUT_MS = Number(process.env.SIDECAR_NAV_TIMEOUT_MS || 20000);
const ACT_TIMEOUT_MS = Number(process.env.SIDECAR_ACT_TIMEOUT_MS || 15000);

// Verbatim copy of pilot-core/src/driver.rs GEOMETRY_SNAPSHOT_SCRIPT. Keep in
// sync if that constant changes (it stamps data-theorem-id="t{i}" and returns
// {handle, role, name, value, test_id, rect, visible, enabled, editable}).
const GEOMETRY_SNAPSHOT_SCRIPT = `
(function () {
  var EDITABLE_TYPES = ["text","search","email","password","url","tel","number","date","datetime-local","month","time","week"];
  function roleOf(el) {
    var explicit = el.getAttribute("role");
    if (explicit) return explicit.toLowerCase();
    var tag = el.tagName.toLowerCase();
    if (tag === "a" && el.hasAttribute("href")) return "link";
    if (tag === "button") return "button";
    if (tag === "textarea") return "textbox";
    if (tag === "select") return "select";
    if (tag === "input") return (el.getAttribute("type") || "text").toLowerCase();
    return tag;
  }
  function textOf(el) {
    return [
      el.getAttribute("aria-label"),
      el.getAttribute("title"),
      el.getAttribute("placeholder"),
      el.getAttribute("alt"),
      el.textContent
    ].filter(Boolean).join(" ").trim();
  }
  function visibleOf(el, rect) {
    if (!rect || rect.width <= 0 || rect.height <= 0) return false;
    var style = window.getComputedStyle(el);
    if (style.display === "none" || style.visibility === "hidden") return false;
    if (el.offsetParent === null && style.position !== "fixed") return false;
    return true;
  }
  var nodes = document.querySelectorAll("a[href],button,input,select,textarea,[role],[data-testid]");
  var out = [];
  for (var i = 0; i < nodes.length; i++) {
    var el = nodes[i];
    var handle = "t" + i;
    el.setAttribute("data-theorem-id", handle);
    var r = el.getBoundingClientRect();
    var tag = el.tagName.toLowerCase();
    var type = tag === "input" ? (el.getAttribute("type") || "text").toLowerCase() : "";
    var disabled = el.disabled === true || el.getAttribute("aria-disabled") === "true";
    var readonly = el.readOnly === true || el.getAttribute("aria-readonly") === "true";
    var editable = (tag === "textarea" || (tag === "input" && EDITABLE_TYPES.indexOf(type) !== -1)) && !readonly;
    out.push({
      handle: handle,
      role: roleOf(el),
      name: textOf(el),
      value: el.value !== undefined ? el.value : null,
      test_id: el.getAttribute("data-testid") || el.getAttribute("data-test-id") || el.getAttribute("data-test") || null,
      rect: { x: r.x, y: r.y, w: r.width, h: r.height },
      visible: visibleOf(el, r),
      enabled: !disabled,
      editable: editable,
      degraded: false
    });
  }
  return JSON.stringify(out);
})()
`;

let browser;
const sessions = new Map(); // session_id -> { context, page }

async function getBrowser() {
  if (!browser) {
    browser = await chromium.launch({ args: ["--no-sandbox"] });
  }
  return browser;
}

function round(n) {
  return Math.round(Number(n) || 0);
}

// Build a serde-compatible pilot-core PageState from a live Playwright page.
async function buildPageState(page) {
  let raw = "[]";
  try {
    raw = await page.evaluate(GEOMETRY_SNAPSHOT_SCRIPT);
  } catch {
    raw = "[]";
  }
  let arr = [];
  try {
    arr = JSON.parse(raw);
  } catch {
    arr = [];
  }
  const interactive_elements = arr.map((e) => ({
    element_id: e.handle,
    role: e.role || "",
    name: e.name || "",
    value: e.value && String(e.value).length ? String(e.value) : null,
    test_id: e.test_id && String(e.test_id).length ? String(e.test_id) : null,
    bbox: e.rect
      ? { x: round(e.rect.x), y: round(e.rect.y), width: round(e.rect.w), height: round(e.rect.h) }
      : null,
    visible: !!e.visible,
    enabled: e.enabled !== false,
    editable: !!e.editable,
    degraded: !!e.degraded,
  }));
  let distilled_text = "";
  try {
    distilled_text = await page.evaluate(() => (document.body ? document.body.innerText : ""));
  } catch {
    distilled_text = "";
  }
  let title = "";
  try {
    title = await page.title();
  } catch {
    title = "";
  }
  return {
    url: page.url(),
    title,
    distilled_text: (distilled_text || "").slice(0, DISTILLED_MAX),
    interactive_elements,
    active_tab_id: null,
    fetch: null,
  };
}

async function screenshotPayload(page, include) {
  if (!include) return {};
  const bytes = await page.screenshot({ type: "png" });
  return {
    screenshot_base64: bytes.toString("base64"),
    screenshot_media_type: "image/png",
  };
}

function finitePoint(kind) {
  const point = kind && kind.point;
  const x = Number(point && point.x);
  const y = Number(point && point.y);
  if (!Number.isFinite(x) || !Number.isFinite(y)) return null;
  return { x, y };
}

const app = express();
app.use(express.json({ limit: "8mb" }));

app.get("/healthz", (_req, res) => res.json({ ok: true, sessions: sessions.size }));

// ---- Surface 1: rendered fetch -------------------------------------------
app.post("/render", async (req, res) => {
  const { url, max_bytes } = req.body || {};
  if (!url) return res.status(400).json({ error: "missing url" });
  let context;
  try {
    context = await (await getBrowser()).newContext();
    const page = await context.newPage();
    await page.goto(url, { waitUntil: "domcontentloaded", timeout: NAV_TIMEOUT_MS });
    let html = await page.content();
    if (max_bytes && Number(max_bytes) > 0) html = html.slice(0, Number(max_bytes));
    res.json({
      html,
      http_status: 200,
      content_type: "text/html; charset=utf-8",
      final_url: page.url(),
    });
  } catch (err) {
    res.status(502).json({ error: String(err && err.message ? err.message : err) });
  } finally {
    if (context) await context.close().catch(() => {});
  }
});

// ---- Surface 2: live action loop -----------------------------------------
app.post("/sessions/checkout", async (req, res) => {
  const { run_id, session_id, url, include_screenshot } = req.body || {};
  const id = session_id || `${run_id || "run"}:${Date.now()}`;
  try {
    let entry = sessions.get(id);
    if (!entry) {
      const context = await (await getBrowser()).newContext();
      const page = await context.newPage();
      entry = { context, page };
      sessions.set(id, entry);
    }
    if (url) {
      await entry.page.goto(url, { waitUntil: "domcontentloaded", timeout: NAV_TIMEOUT_MS });
    }
    res.json({
      session_id: id,
      page: await buildPageState(entry.page),
      ...(await screenshotPayload(entry.page, !!include_screenshot)),
    });
  } catch (err) {
    res.status(502).json({ error: String(err && err.message ? err.message : err) });
  }
});

app.post("/sessions/snapshot", async (req, res) => {
  const { session_id, include_screenshot } = req.body || {};
  const entry = sessions.get(session_id);
  if (!entry) return res.status(404).json({ error: "unknown session_id" });
  try {
    res.json({
      page: await buildPageState(entry.page),
      ...(await screenshotPayload(entry.page, !!include_screenshot)),
    });
  } catch (err) {
    res.status(502).json({ error: String(err && err.message ? err.message : err) });
  }
});

app.post("/sessions/actuate", async (req, res) => {
  const { session_id, plan, include_screenshot } = req.body || {};
  const entry = sessions.get(session_id);
  if (!entry) return res.status(404).json({ error: "unknown session_id" });
  const handle = plan && plan.target_handle;
  const kind = (plan && plan.kind) || {};
  const mech = kind.mechanism;
  const page = entry.page;
  const loc = handle ? page.locator(`[data-theorem-id="${handle}"]`) : null;
  let mechanism = "sidecar.unsupported";
  try {
    if (mech === "coordinate_synthesis") {
      const point = finitePoint(kind);
      const pointer = kind.pointer || "click";
      if (point) {
        if (pointer === "hover") {
          await page.mouse.move(point.x, point.y);
        } else if (pointer === "double_click") {
          await page.mouse.dblclick(point.x, point.y);
        } else {
          await page.mouse.click(point.x, point.y);
        }
        mechanism = `sidecar.${pointer}`;
      } else {
        await loc.click({ timeout: ACT_TIMEOUT_MS });
        mechanism = "sidecar.click";
      }
    } else if (mech === "keyboard") {
      const point = finitePoint(kind);
      if (point && (!handle || String(handle).startsWith("visual:"))) {
        await page.mouse.click(point.x, point.y);
        await page.keyboard.type(kind.text != null ? String(kind.text) : "");
        mechanism = "sidecar.coordinate_keyboard";
      } else if (loc) {
        await loc.fill(kind.text != null ? String(kind.text) : "", { timeout: ACT_TIMEOUT_MS });
        mechanism = "sidecar.fill";
      } else if (point) {
        await page.mouse.click(point.x, point.y);
        await page.keyboard.type(kind.text != null ? String(kind.text) : "");
        mechanism = "sidecar.coordinate_keyboard";
      } else {
        throw new Error("keyboard actuation requires a locator or point");
      }
    } else if (mech === "scroll") {
      await loc.scrollIntoViewIfNeeded({ timeout: ACT_TIMEOUT_MS });
      mechanism = "sidecar.scroll";
    } else {
      // embedder_control / semantic_activation are V1-unsupported by this
      // sidecar; report honestly rather than 5xx so the run can proceed.
      return res.json({
        receipt: { mechanism, detail: { target: handle || null, requested_mechanism: mech || null } },
        page: await buildPageState(page),
        ...(await screenshotPayload(page, !!include_screenshot)),
      });
    }
    // Best-effort settle so the post-action snapshot reflects navigation.
    await page.waitForLoadState("domcontentloaded", { timeout: 3000 }).catch(() => {});
    res.json({
      receipt: { mechanism, detail: { target: handle, requested_mechanism: mech } },
      page: await buildPageState(page),
      ...(await screenshotPayload(page, !!include_screenshot)),
    });
  } catch (err) {
    res.status(502).json({ error: String(err && err.message ? err.message : err) });
  }
});

app.listen(PORT, () => {
  console.log(`[theorem-browser-sidecar] listening on :${PORT}`);
  console.log(`  render endpoint     -> POST http://127.0.0.1:${PORT}/render`);
  console.log(`  live action loop    -> POST http://127.0.0.1:${PORT}/sessions/{checkout,snapshot,actuate}`);
});

async function shutdown() {
  for (const { context } of sessions.values()) await context.close().catch(() => {});
  if (browser) await browser.close().catch(() => {});
  process.exit(0);
}
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
