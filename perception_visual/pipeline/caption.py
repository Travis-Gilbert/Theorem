"""Florence-2 caption wrapper (MIT).

Lazy: the Florence-2 weights and transformers/torch load on first use. No-op
(leaves element content unchanged) when transformers/torch are absent, so the
pipeline still runs without the caption stack.
"""
from __future__ import annotations

import logging

import numpy as np

logger = logging.getLogger(__name__)
_model = None
_processor = None
_MODEL_ID = "microsoft/Florence-2-base"  # MIT
_MODEL_REVISION = "dc7e6928b12c148726d4cee0ff011cfe2cebddea"


def _load():
    global _model, _processor
    if _model is not None:
        return _model, _processor
    from transformers import AutoModelForCausalLM, AutoProcessor  # type: ignore  # MIT (Florence-2)

    # trust_remote_code executes model-repo Python during load; pin the revision
    # so deploys do not silently track upstream code changes.
    _model = AutoModelForCausalLM.from_pretrained(
        _MODEL_ID, revision=_MODEL_REVISION, trust_remote_code=True
    )
    _processor = AutoProcessor.from_pretrained(
        _MODEL_ID, revision=_MODEL_REVISION, trust_remote_code=True
    )
    _model.eval()
    return _model, _processor


def caption_elements(image, elements, prompt: str = "<CAPTION>") -> None:
    """Fill `content` on icon elements with a Florence-2 caption per crop.

    Mutates `elements` in place. No-op if Florence-2 / transformers are absent.
    """
    try:
        model, processor = _load()
    except Exception as exc:  # pragma: no cover - no transformers in CI
        logger.info("Captioning disabled (Florence-2 unavailable): %s", exc)
        return

    from PIL import Image  # pragma: no cover - needs caption stack

    arr = np.asarray(image)[:, :, :3].astype("uint8")
    h, w = arr.shape[:2]
    for el in elements:  # pragma: no cover - needs caption stack
        if el.source != "icon":
            continue
        x1, y1 = int(el.box.x * w), int(el.box.y * h)
        x2, y2 = int(el.box.x2 * w), int(el.box.y2 * h)
        if x2 <= x1 or y2 <= y1:
            continue
        crop = Image.fromarray(arr[y1:y2, x1:x2])
        inputs = processor(text=prompt, images=crop, return_tensors="pt")
        generated = model.generate(**inputs, max_new_tokens=64, num_beams=1)
        text = processor.batch_decode(generated, skip_special_tokens=True)[0].strip()
        el.content = text
