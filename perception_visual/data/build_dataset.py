"""Accessibility-tree -> COCO detection dataset.

Drives a Chromium context (Playwright) over a URL list and, for each page,
reads the interactable elements (role + bounding client rect) from the DOM /
accessibility tree, then emits a COCO-format detection dataset (image + boxes,
single class "interactable"). This produces license-clean training data from
rendered pages and depends on no OmniParser dataset.

The DOM extraction and COCO assembly are pure functions (testable without a
browser); only `build_from_urls` needs Playwright, imported lazily.
"""
from __future__ import annotations

import argparse
import json
import logging
from pathlib import Path

logger = logging.getLogger(__name__)

CATEGORY_ID = 1
CATEGORY_NAME = "interactable"

# Roles/tags treated as interactable. The in-page collector pre-filters to
# these; we re-filter here so the pure path is independently testable.
INTERACTABLE_ROLES = {
    "a",
    "button",
    "input",
    "select",
    "textarea",
    "summary",
    "label",
    "link",
    "checkbox",
    "radio",
    "switch",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "tab",
    "slider",
    "spinbutton",
    "combobox",
    "textbox",
    "searchbox",
    "option",
}

# Collected in-page: every interactable element's role + client rect + viewport.
_COLLECT_JS = """
() => {
  const sel = 'a[href], button, input, select, textarea, summary, label, [role], [onclick], [tabindex]';
  const out = [];
  for (const el of document.querySelectorAll(sel)) {
    const r = el.getBoundingClientRect();
    if (r.width < 4 || r.height < 4) continue;
    if (r.bottom < 0 || r.right < 0 || r.top > innerHeight || r.left > innerWidth) continue;
    const s = getComputedStyle(el);
    if (s.visibility === 'hidden' || s.display === 'none' || s.opacity === '0') continue;
    const role = el.getAttribute('role') || el.tagName.toLowerCase();
    out.push({role, x: r.left, y: r.top, width: r.width, height: r.height});
  }
  return {viewport: {width: innerWidth, height: innerHeight}, elements: out};
}
"""


def extract_interactables(payload: dict, min_size: float = 4.0) -> list[dict]:
    """Filter a collected page payload to clean interactable boxes.

    `payload` = {"viewport": {"width", "height"}, "elements": [{role, x, y,
    width, height}]}. Returns [{"role", "bbox": [x, y, w, h]}] in absolute
    pixels, clipped to the viewport, role-filtered, size-filtered.
    """
    viewport = payload.get("viewport", {})
    vw = float(viewport.get("width", 0) or 0)
    vh = float(viewport.get("height", 0) or 0)
    out: list[dict] = []
    for el in payload.get("elements", []):
        role = str(el.get("role", "")).lower()
        if role not in INTERACTABLE_ROLES:
            continue
        x, y = float(el.get("x", 0)), float(el.get("y", 0))
        w, h = float(el.get("width", 0)), float(el.get("height", 0))
        if w < min_size or h < min_size:
            continue
        # Clip to viewport.
        if vw and vh:
            x1, y1 = max(0.0, x), max(0.0, y)
            x2, y2 = min(vw, x + w), min(vh, y + h)
            if x2 - x1 < min_size or y2 - y1 < min_size:
                continue
            x, y, w, h = x1, y1, x2 - x1, y2 - y1
        out.append({"role": role, "bbox": [x, y, w, h]})
    return out


def to_coco(samples: list[dict]) -> dict:
    """Assemble a COCO detection dict from per-image samples.

    Each sample: {"file_name", "width", "height", "boxes": [[x, y, w, h], ...]}.
    """
    images, annotations = [], []
    ann_id = 1
    for img_id, sample in enumerate(samples, start=1):
        images.append(
            {
                "id": img_id,
                "file_name": sample["file_name"],
                "width": int(sample["width"]),
                "height": int(sample["height"]),
            }
        )
        for box in sample.get("boxes", []):
            x, y, w, h = (float(v) for v in box)
            annotations.append(
                {
                    "id": ann_id,
                    "image_id": img_id,
                    "category_id": CATEGORY_ID,
                    "bbox": [x, y, w, h],
                    "area": w * h,
                    "iscrowd": 0,
                }
            )
            ann_id += 1
    return {
        "images": images,
        "annotations": annotations,
        "categories": [{"id": CATEGORY_ID, "name": CATEGORY_NAME}],
    }


def build_from_urls(
    urls: list[str],
    out_dir: str | Path,
    *,
    viewport=(1280, 800),
    timeout_ms: int = 30000,
) -> dict:  # pragma: no cover - needs Playwright + Chromium
    """Render each URL, collect interactables, write images + coco.json.

    Returns the COCO dict. Requires Playwright (`pip install playwright` then
    `playwright install chromium`), imported lazily.
    """
    try:
        from playwright.sync_api import sync_playwright  # type: ignore
    except Exception as exc:
        raise ImportError(
            "build_from_urls requires Playwright (pip install playwright && "
            "playwright install chromium)."
        ) from exc

    out_dir = Path(out_dir)
    images_dir = out_dir / "images"
    images_dir.mkdir(parents=True, exist_ok=True)

    samples: list[dict] = []
    with sync_playwright() as pw:
        browser = pw.chromium.launch(headless=True)
        page = browser.new_page(viewport={"width": viewport[0], "height": viewport[1]})
        for i, url in enumerate(urls):
            try:
                page.goto(url, timeout=timeout_ms, wait_until="networkidle")
            except Exception as exc:
                logger.warning("skip %s: %s", url, exc)
                continue
            file_name = f"page_{i:05d}.png"
            page.screenshot(path=str(images_dir / file_name), full_page=False)
            payload = page.evaluate(_COLLECT_JS)
            boxes = [b["bbox"] for b in extract_interactables(payload)]
            samples.append(
                {
                    "file_name": file_name,
                    "width": viewport[0],
                    "height": viewport[1],
                    "boxes": boxes,
                }
            )
        browser.close()

    coco = to_coco(samples)
    (out_dir / "coco.json").write_text(json.dumps(coco, indent=2))
    logger.info("wrote %d images, %d boxes -> %s", len(samples), len(coco["annotations"]), out_dir)
    return coco


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Build a COCO interactable-element dataset from URLs.")
    ap.add_argument("--urls", nargs="*", default=[], help="URLs to render")
    ap.add_argument("--urls-file", help="file with one URL per line")
    ap.add_argument("--out", required=True, help="output dataset directory")
    ap.add_argument("--width", type=int, default=1280)
    ap.add_argument("--height", type=int, default=800)
    args = ap.parse_args(argv)

    urls = list(args.urls)
    if args.urls_file:
        urls += [ln.strip() for ln in Path(args.urls_file).read_text().splitlines() if ln.strip()]
    if not urls:
        ap.error("provide --urls or --urls-file")

    coco = build_from_urls(urls, args.out, viewport=(args.width, args.height))
    print(f"dataset: {len(coco['images'])} images, {len(coco['annotations'])} boxes -> {args.out}")
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
