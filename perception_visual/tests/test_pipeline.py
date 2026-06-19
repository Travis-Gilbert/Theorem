"""Tests for perception_visual.

Designed to run on numpy + stdlib alone; tests needing Pillow or FastAPI use
`pytest.importorskip` so they run where those (light) deps are installed and
skip cleanly otherwise. The heavy model stack (rfdetr/transformers/easyocr) is
never required.
"""
from __future__ import annotations

import base64
from pathlib import Path

import numpy as np
import pytest

from perception_visual.data.build_dataset import extract_interactables, to_coco
from perception_visual.detector.base import Box, Detector, intersection_over_smaller, iou
from perception_visual.detector.heuristic_detector import HeuristicDetector
from perception_visual.eval.screenspot_eval import (
    evaluate,
    grounding_accuracy,
    grounding_hits,
)
from perception_visual.pipeline.som import (
    LabeledElement,
    annotate,
    label_elements,
    merge_boxes,
    nms,
    parse,
    remove_overlapping_with_text,
)
from perception_visual.train.finetune import validate_coco_dataset

PKG_ROOT = Path(__file__).resolve().parents[1]
SCREENSHOT = PKG_ROOT / "tests" / "fixtures" / "screenshot.png"
HELDOUT = PKG_ROOT / "eval" / "fixtures" / "heldout"


# --- box geometry ------------------------------------------------------------


def test_iou_identical_and_disjoint():
    a = Box(0.0, 0.0, 0.5, 0.5, 1.0)
    assert iou(a, a) == pytest.approx(1.0)
    b = Box(0.6, 0.6, 0.3, 0.3, 1.0)
    assert iou(a, b) == 0.0


def test_intersection_over_smaller_contained():
    big = Box(0.0, 0.0, 1.0, 1.0, 1.0)
    small = Box(0.4, 0.4, 0.1, 0.1, 1.0)
    # small fully inside big -> fully covered
    assert intersection_over_smaller(big, small) == pytest.approx(1.0)
    assert iou(big, small) < 0.05  # IoU stays low; that's why we use IoS


# --- SOM merge / label -------------------------------------------------------


def test_nms_suppresses_overlapping_lower_score():
    high = Box(0.0, 0.0, 0.5, 0.5, 0.9)
    dup = Box(0.01, 0.01, 0.5, 0.5, 0.4)  # ~same box, lower score
    far = Box(0.7, 0.7, 0.2, 0.2, 0.8)
    kept = nms([high, dup, far], iou_thresh=0.7)
    assert high in kept and far in kept and dup not in kept


def test_remove_icon_overlapping_text():
    icon = Box(0.30, 0.30, 0.08, 0.05, 0.9)
    text = Box(0.28, 0.28, 0.20, 0.10, 0.9, label="text")
    assert remove_overlapping_with_text([icon], [text], cover_thresh=0.7) == []


def test_label_elements_orders_text_then_icons_with_sequential_ids():
    icons = [Box(0.7, 0.1, 0.05, 0.05, 0.9, label="icon")]
    texts = [(Box(0.1, 0.1, 0.2, 0.05, 0.95, label="text"), "Submit")]
    elements = label_elements(icons, texts)
    assert [e.source for e in elements] == ["ocr", "icon"]
    assert [e.id for e in elements] == [0, 1]
    assert elements[0].content == "Submit"


def test_merge_boxes_drops_icon_under_text():
    icons = [Box(0.30, 0.30, 0.08, 0.05, 0.9)]
    texts = [Box(0.28, 0.28, 0.20, 0.10, 0.9)]
    kept_icons, kept_texts = merge_boxes(icons, texts)
    assert kept_icons == [] and len(kept_texts) == 1


# --- detector ----------------------------------------------------------------


def test_heuristic_detector_is_a_detector_and_finds_regions():
    det = HeuristicDetector()
    assert isinstance(det, Detector)  # runtime_checkable protocol
    img = _load_screenshot()
    boxes = det.detect(img)
    assert boxes, "heuristic detector should propose regions on the fixture"
    assert all(0.0 <= b.x <= 1.0 and 0.0 <= b.y <= 1.0 for b in boxes)


# --- parse() end to end ------------------------------------------------------


def test_parse_returns_annotated_png_and_elements():
    img = _load_screenshot()
    annotated, elements = parse(img, detector=HeuristicDetector(), use_ocr=False)
    assert isinstance(annotated, (bytes, bytearray))
    assert annotated[:8] == b"\x89PNG\r\n\x1a\n"  # valid PNG signature
    assert elements and all(isinstance(e, LabeledElement) for e in elements)
    d = elements[0].to_dict((256, 256))
    assert set(d["box"]) == {"x", "y", "w", "h"} and "box_pixels" in d


def test_annotate_without_pillow_still_writes_png(monkeypatch):
    # Force the stdlib PNG fallback even if Pillow is installed.
    import builtins

    real_import = builtins.__import__

    def no_pil(name, *args, **kwargs):
        if name == "PIL" or name.startswith("PIL."):
            raise ImportError("PIL disabled for test")
        return real_import(name, *args, **kwargs)

    monkeypatch.setattr(builtins, "__import__", no_pil)
    img = np.full((64, 64, 3), 20, np.uint8)
    el = LabeledElement(0, Box(0.1, 0.1, 0.3, 0.3, 0.9), True, "icon")
    png = annotate(img, [el])
    assert png[:8] == b"\x89PNG\r\n\x1a\n"


# --- dataset build (pure) ----------------------------------------------------


def test_extract_interactables_filters_role_and_size():
    payload = {
        "viewport": {"width": 1000, "height": 800},
        "elements": [
            {"role": "button", "x": 10, "y": 10, "width": 50, "height": 20},
            {"role": "div", "x": 0, "y": 0, "width": 100, "height": 100},  # not interactable
            {"role": "a", "x": 5, "y": 5, "width": 2, "height": 2},  # too small
            {"role": "textbox", "x": 990, "y": 10, "width": 40, "height": 20},  # clipped to viewport
        ],
    }
    out = extract_interactables(payload)
    roles = sorted(o["role"] for o in out)
    assert roles == ["button", "textbox"]
    # textbox is clipped to the 1000px-wide viewport
    tb = next(o for o in out if o["role"] == "textbox")
    assert tb["bbox"][0] + tb["bbox"][2] <= 1000


def test_to_coco_shape():
    samples = [{"file_name": "a.png", "width": 100, "height": 80, "boxes": [[1, 2, 3, 4]]}]
    coco = to_coco(samples)
    assert coco["categories"] == [{"id": 1, "name": "interactable"}]
    assert coco["images"][0]["file_name"] == "a.png"
    assert coco["annotations"][0]["bbox"] == [1.0, 2.0, 3.0, 4.0]
    assert coco["annotations"][0]["area"] == 12.0


# --- train dataset validation ------------------------------------------------


def test_validate_coco_dataset(tmp_path):
    import json

    (tmp_path / "images").mkdir()
    (tmp_path / "images" / "a.png").write_bytes(b"not-a-real-png-but-present")
    coco = to_coco([{"file_name": "a.png", "width": 10, "height": 10, "boxes": [[0, 0, 5, 5]]}])
    (tmp_path / "coco.json").write_text(json.dumps(coco))
    stats = validate_coco_dataset(tmp_path)
    assert stats["images"] == 1 and stats["annotations"] == 1 and not stats["missing_files"]


def test_validate_coco_dataset_flags_missing(tmp_path):
    import json

    (tmp_path / "images").mkdir()
    coco = to_coco([{"file_name": "missing.png", "width": 10, "height": 10, "boxes": []}])
    (tmp_path / "coco.json").write_text(json.dumps(coco))
    stats = validate_coco_dataset(tmp_path)
    assert stats["missing_files"] == ["missing.png"]


# --- eval metric -------------------------------------------------------------


def test_grounding_hits_center_in_box():
    box = Box(0.1, 0.1, 0.2, 0.2, 0.9)  # covers center (0.2, 0.2)
    assert grounding_hits([box], [(0.2, 0.2)]) == 1
    assert grounding_hits([box], [(0.8, 0.8)]) == 0


def test_grounding_accuracy_aggregate():
    box = Box(0.0, 0.0, 0.5, 0.5, 0.9)
    report = grounding_accuracy([([box], [(0.1, 0.1), (0.9, 0.9)])])
    assert report["total"] == 2 and report["hits"] == 1
    assert report["accuracy"] == pytest.approx(0.5)


def test_evaluate_heldout_split_runs_and_scores():
    pytest.importorskip("PIL")  # evaluate reads PNGs via Pillow
    report = evaluate(HELDOUT, detector=HeuristicDetector())
    assert report["images"] == 5 and report["total"] == 18
    assert 0.0 <= report["accuracy"] <= 1.0
    assert report["hits"] > 0  # the harness localizes at least some planted icons


# --- license gate (acceptance: no ultralytics anywhere) ----------------------


def test_no_ultralytics_import_in_package():
    # Build the forbidden token at runtime so this scanner does not match its
    # own source; the scan then covers every file, tests included.
    forbidden = "ultra" + "lytics"
    markers = (f"import {forbidden}", f"from {forbidden}", f"{forbidden}.")
    offenders = []
    for path in PKG_ROOT.rglob("*.py"):
        if ".venv" in path.parts:
            continue
        text = path.read_text(encoding="utf-8")
        for marker in markers:
            if marker in text:
                offenders.append(f"{path.relative_to(PKG_ROOT)}: {marker}")
    assert not offenders, f"AGPL detector library referenced: {offenders}"


# --- serving -----------------------------------------------------------------


def test_parse_endpoint_returns_json_elements():
    pytest.importorskip("fastapi")
    pytest.importorskip("PIL")
    from fastapi.testclient import TestClient

    from perception_visual.serve.app import app

    client = TestClient(app)
    img_b64 = base64.b64encode(SCREENSHOT.read_bytes()).decode()
    resp = client.post("/parse", json={"image_base64": img_b64, "use_ocr": False})
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert body["image_size"] == {"width": 256, "height": 256}
    assert isinstance(body["elements"], list) and body["count"] == len(body["elements"])
    assert "annotated_image_base64" in body


def test_health_endpoint():
    pytest.importorskip("fastapi")
    from fastapi.testclient import TestClient

    from perception_visual.serve.app import app

    resp = TestClient(app).get("/health")
    assert resp.status_code == 200 and resp.json()["status"] == "ok"


# --- helpers -----------------------------------------------------------------


def _load_screenshot() -> np.ndarray:
    """Load the fixture screenshot as an RGB array (Pillow if present, else stdlib PNG)."""
    try:
        from PIL import Image

        return np.asarray(Image.open(SCREENSHOT).convert("RGB"))
    except ImportError:
        return _decode_png_stdlib(SCREENSHOT.read_bytes())


def _decode_png_stdlib(data: bytes) -> np.ndarray:
    """Tiny stdlib PNG decoder for the fixture (8-bit RGB, filter 0) -- test-only."""
    import struct
    import zlib

    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    pos = 8
    width = height = 0
    idat = b""
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        ctype = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + length]
        if ctype == b"IHDR":
            width, height = struct.unpack(">II", chunk[:8])
        elif ctype == b"IDAT":
            idat += chunk
        pos += 12 + length
    raw = zlib.decompress(idat)
    stride = width * 3
    out = np.zeros((height, width, 3), np.uint8)
    i = 0
    for y in range(height):
        i += 1  # skip filter byte (fixtures use filter 0)
        out[y] = np.frombuffer(raw[i : i + stride], np.uint8).reshape(width, 3)
        i += stride
    return out
