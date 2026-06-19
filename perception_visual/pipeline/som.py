"""Set-of-Mark (SOM) labeling pipeline, de-Ultralytics-ed.

This is the vendored, rewritten `get_som_labeled_img` path from OmniParser
(MIT). The only structural change versus upstream is the detector seam: where
OmniParser called a fine-tuned Ultralytics YOLOv8 (AGPL), this calls
`Detector.detect` (RF-DETR by default). The OCR overlap merge and the optional
Florence-2 caption step are kept.

`parse(image)` mirrors OmniParser's public entry: it returns the annotated
image (PNG bytes) plus the structured element list.
"""
from __future__ import annotations

import logging
import warnings
from dataclasses import dataclass

import numpy as np

from ..detector.base import Box, Detector, intersection_over_smaller, iou

logger = logging.getLogger(__name__)


@dataclass
class LabeledElement:
    """One marked, addressable element in the parsed screen."""

    id: int
    box: Box  # normalized [0, 1]
    interactable: bool
    source: str  # "ocr" | "icon"
    content: str = ""  # OCR text or icon caption

    def to_dict(self, image_size: tuple[int, int] | None = None) -> dict:
        out = {
            "id": self.id,
            "interactable": self.interactable,
            "source": self.source,
            "content": self.content,
            "score": self.box.score,
            "box": {"x": self.box.x, "y": self.box.y, "w": self.box.w, "h": self.box.h},
        }
        if image_size is not None:
            w, h = image_size
            out["box_pixels"] = {
                "x1": round(self.box.x * w),
                "y1": round(self.box.y * h),
                "x2": round(self.box.x2 * w),
                "y2": round(self.box.y2 * h),
            }
        return out


# --- box merging (pure) ------------------------------------------------------


def nms(boxes: list[Box], iou_thresh: float = 0.7) -> list[Box]:
    """Greedy non-max suppression by score, IoU-thresholded."""
    kept: list[Box] = []
    for b in sorted(boxes, key=lambda b: b.score, reverse=True):
        if all(iou(b, k) < iou_thresh for k in kept):
            kept.append(b)
    return kept


def remove_overlapping_with_text(
    icons: list[Box], texts: list[Box], cover_thresh: float = 0.7
) -> list[Box]:
    """Drop icon boxes that sit (mostly) inside a text box.

    OmniParser does this so an OCR'd label and the icon behind it do not both
    become separate marks. Uses intersection-over-smaller, not IoU, because the
    icon and its text label often differ a lot in size.
    """
    return [
        icon
        for icon in icons
        if not any(intersection_over_smaller(icon, t) > cover_thresh for t in texts)
    ]


def merge_boxes(
    icon_boxes: list[Box],
    text_boxes: list[Box],
    iou_thresh: float = 0.7,
    cover_thresh: float = 0.7,
) -> tuple[list[Box], list[Box]]:
    """Return (kept_icons, kept_texts) after NMS + text-overlap removal."""
    texts = nms(text_boxes, iou_thresh=iou_thresh)
    icons = nms(icon_boxes, iou_thresh=iou_thresh)
    icons = remove_overlapping_with_text(icons, texts, cover_thresh=cover_thresh)
    return icons, texts


def label_elements(
    icon_boxes: list[Box],
    text_results: list[tuple[Box, str]] | None = None,
    iou_thresh: float = 0.7,
    cover_thresh: float = 0.7,
) -> list[LabeledElement]:
    """Assemble the final marked element list from detector + OCR output.

    Order follows OmniParser: OCR (text) marks first, then icon marks; ids are
    assigned sequentially over that order.
    """
    text_results = text_results or []
    text_boxes = [b for b, _ in text_results]
    text_content = {id(b): t for b, t in text_results}
    icons, texts = merge_boxes(icon_boxes, text_boxes, iou_thresh, cover_thresh)

    elements: list[LabeledElement] = []
    idx = 0
    for b in texts:
        elements.append(LabeledElement(idx, b, True, "ocr", text_content.get(id(b), "")))
        idx += 1
    for b in icons:
        elements.append(LabeledElement(idx, b, b.interactable, "icon", b.label))
        idx += 1
    return elements


# --- annotated overlay -------------------------------------------------------

_PALETTE = [
    (255, 64, 64),
    (64, 160, 255),
    (64, 200, 64),
    (240, 200, 32),
    (200, 64, 240),
]


def annotate(image: np.ndarray, elements: list[LabeledElement]) -> bytes:
    """PNG bytes of the image with one numbered rectangle per element.

    Uses Pillow when available (boxes + numeric labels); otherwise a stdlib PNG
    with box borders only (the numeric labels still live in the JSON).
    """
    arr = np.asarray(image)[:, :, :3].astype(np.uint8).copy()
    h, w = arr.shape[:2]

    try:
        from PIL import Image, ImageDraw  # type: ignore

        has_pil = True
    except ImportError:
        has_pil = False

    if has_pil:
        import io

        img = Image.fromarray(arr)
        draw = ImageDraw.Draw(img)
        for el in elements:
            color = _PALETTE[el.id % len(_PALETTE)]
            x1, y1 = int(el.box.x * w), int(el.box.y * h)
            x2, y2 = int(el.box.x2 * w), int(el.box.y2 * h)
            draw.rectangle([x1, y1, x2, y2], outline=color, width=2)
            draw.text((x1 + 2, max(0, y1 + 2)), str(el.id), fill=color)
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        return buf.getvalue()

    from ._png import encode_png

    for el in elements:
        color = np.array(_PALETTE[el.id % len(_PALETTE)], dtype=np.uint8)
        x1 = max(0, min(w - 1, int(el.box.x * w)))
        y1 = max(0, min(h - 1, int(el.box.y * h)))
        x2 = max(0, min(w - 1, int(el.box.x2 * w)))
        y2 = max(0, min(h - 1, int(el.box.y2 * h)))
        arr[y1 : y2 + 1, x1] = color
        arr[y1 : y2 + 1, x2] = color
        arr[y1, x1 : x2 + 1] = color
        arr[y2, x1 : x2 + 1] = color
    return encode_png(arr)


# --- public entry ------------------------------------------------------------


def default_detector() -> Detector:
    """RF-DETR in production; HeuristicDetector offline.

    Resolves to the Apache-2.0 RF-DETR detector when `rfdetr` is importable (the
    production path the acceptance criteria name); falls back to the numpy-only
    HeuristicDetector with a warning so `parse` still runs where the model stack
    is absent (CI, this environment).
    """
    from ..detector.heuristic_detector import HeuristicDetector
    from ..detector.rfdetr_detector import RfDetrDetector

    det = RfDetrDetector()
    try:
        det._load()  # force the lazy import now to learn if rfdetr is present
        return det
    except ImportError:
        warnings.warn(
            "rfdetr not installed; parse() falls back to the offline "
            "HeuristicDetector (not an accuracy path). Install rfdetr for the "
            "production detector.",
            RuntimeWarning,
            stacklevel=2,
        )
        return HeuristicDetector()


def parse(
    image,
    *,
    detector: Detector | None = None,
    use_ocr: bool = True,
    caption: bool = False,
) -> tuple[bytes, list[LabeledElement]]:
    """Screenshot in -> (annotated PNG bytes, [LabeledElement]).

    Mirrors OmniParser's `parse`. `detector` defaults to RF-DETR (offline
    fallback to HeuristicDetector). OCR (easyocr) and `caption` (Florence-2) are
    optional and lazy; each degrades to empty results if its package is absent.
    """
    arr = np.asarray(image)
    if arr.ndim == 2:
        arr = np.stack([arr] * 3, axis=-1)

    det = detector or default_detector()
    icon_boxes = det.detect(arr)

    text_results: list[tuple[Box, str]] = []
    if use_ocr:
        from .ocr import run_ocr

        text_results = run_ocr(arr)

    elements = label_elements(icon_boxes, text_results)

    if caption:
        from .caption import caption_elements

        caption_elements(arr, elements)

    annotated = annotate(arr, elements)
    return annotated, elements
