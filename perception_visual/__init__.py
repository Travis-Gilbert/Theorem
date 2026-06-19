"""perception_visual: license-clean visual GUI perception (OmniParser without AGPL).

Screenshot in -> labeled interactable elements (+ optional captions) out, with
OmniParser's AGPL Ultralytics YOLOv8 detector replaced by Apache-2.0 RF-DETR.
The OmniParser SOM pipeline (MIT) and Florence-2 captioner (MIT) are kept; the
OCR path uses easyocr (Apache-2.0). See LICENSES.md and README.md.

The top-level import is dependency-light (numpy + stdlib). Heavy model stacks
(rfdetr, transformers/torch, easyocr, playwright) are imported lazily by the
function that needs them, so this package imports and its core tests run with
no model installed.
"""
from __future__ import annotations

__version__ = "0.1.0"

from .detector.base import Box, Detector
from .pipeline.som import LabeledElement, parse

__all__ = ["Box", "Detector", "LabeledElement", "parse", "__version__"]
