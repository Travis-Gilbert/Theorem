"""easyocr wrapper (Apache-2.0).

Lazy: the easyocr reader (and its torch backend) load on first use. Degrades to
an empty result when easyocr is not installed, so the pipeline still runs.
"""
from __future__ import annotations

import logging

import numpy as np

from ..detector.base import Box

logger = logging.getLogger(__name__)
_reader = None


def _get_reader(langs=("en",)):
    global _reader
    if _reader is not None:
        return _reader
    import easyocr  # type: ignore  # Apache-2.0

    _reader = easyocr.Reader(list(langs), gpu=False)
    return _reader


def run_ocr(image) -> list[tuple[Box, str]]:
    """Return [(normalized text Box, text)]; [] if easyocr is unavailable."""
    arr = np.asarray(image)
    h, w = arr.shape[:2]
    try:
        reader = _get_reader()
    except Exception as exc:  # pragma: no cover - no easyocr in CI
        logger.info("OCR disabled (easyocr unavailable): %s", exc)
        return []

    results: list[tuple[Box, str]] = []
    for bbox, text, conf in reader.readtext(arr):  # pragma: no cover - needs easyocr
        xs = [float(p[0]) for p in bbox]
        ys = [float(p[1]) for p in bbox]
        x1, y1, x2, y2 = min(xs), min(ys), max(xs), max(ys)
        results.append(
            (
                Box(
                    x=x1 / w,
                    y=y1 / h,
                    w=(x2 - x1) / w,
                    h=(y2 - y1) / h,
                    score=float(conf),
                    interactable=True,
                    label="text",
                ),
                str(text),
            )
        )
    return results
