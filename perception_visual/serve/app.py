"""FastAPI serving endpoint for the Rust theorem-browser-agent perceiver.

`POST /parse` takes a base64 image and returns the structured element list (and
optionally the annotated image) as JSON. Run it with:

    uvicorn perception_visual.serve.app:app --port 8080

In production the detector behind `parse` is RF-DETR; with no model stack
installed the endpoint still answers using the offline HeuristicDetector.
"""
from __future__ import annotations

import base64
import io

import numpy as np
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field

from .. import __version__
from ..pipeline.som import parse

app = FastAPI(title="perception_visual", version=__version__)


class ParseRequest(BaseModel):
    image_base64: str = Field(..., description="base64 image bytes, no data: prefix")
    media_type: str = "image/png"
    use_ocr: bool = True
    caption: bool = False
    include_annotated: bool = True


class HealthResponse(BaseModel):
    status: str
    version: str


def _decode_image(data: bytes) -> np.ndarray:
    from PIL import Image  # serve needs an image decoder

    try:
        img = Image.open(io.BytesIO(data)).convert("RGB")
    except Exception as exc:  # noqa: BLE001
        raise HTTPException(status_code=400, detail=f"undecodable image: {exc}") from exc
    return np.asarray(img)


@app.get("/health", response_model=HealthResponse)
def health() -> HealthResponse:
    return HealthResponse(status="ok", version=__version__)


@app.post("/parse")
def parse_endpoint(req: ParseRequest) -> dict:
    try:
        raw = base64.b64decode(req.image_base64, validate=True)
    except Exception as exc:  # noqa: BLE001
        raise HTTPException(status_code=400, detail=f"invalid base64: {exc}") from exc

    arr = _decode_image(raw)
    annotated, elements = parse(arr, use_ocr=req.use_ocr, caption=req.caption)
    h, w = arr.shape[:2]

    response: dict = {
        "image_size": {"width": int(w), "height": int(h)},
        "elements": [el.to_dict((w, h)) for el in elements],
        "count": len(elements),
    }
    if req.include_annotated:
        response["annotated_image_base64"] = base64.b64encode(annotated).decode()
        response["annotated_media_type"] = "image/png"
    return response
