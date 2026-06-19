"""RF-DETR detector (Roboflow, Apache-2.0) -- the production detector.

RF-DETR's code and its base checkpoint are Apache-2.0 (see LICENSES.md), so it
ships inside the commercial harness, unlike OmniParser's AGPL Ultralytics
YOLOv8. The `rfdetr` package and its model download are imported lazily so the
rest of perception_visual runs without the model stack installed.

The `rfdetr` predict API is mapped here as of the pinned version recorded in
LICENSES.md; verify the call shape against the installed package before relying
on it in production.
"""
from __future__ import annotations

from .base import Box

_DEFAULT_THRESHOLD = 0.3


class RfDetrDetector:
    name = "rf-detr"

    def __init__(
        self,
        checkpoint: str | None = None,
        threshold: float = _DEFAULT_THRESHOLD,
        device: str | None = None,
    ):
        self.checkpoint = checkpoint
        self.threshold = float(threshold)
        self.device = device
        self._model = None

    def _load(self):
        if self._model is not None:
            return self._model
        try:
            from rfdetr import RFDETRBase  # type: ignore  # Apache-2.0
        except Exception as exc:  # pragma: no cover - only with deps installed
            raise ImportError(
                "RfDetrDetector requires the Apache-2.0 'rfdetr' package "
                "(pip install rfdetr). For offline wiring use HeuristicDetector."
            ) from exc
        kwargs: dict = {}
        if self.checkpoint:
            kwargs["pretrain_weights"] = self.checkpoint
        if self.device:
            kwargs["device"] = self.device
        self._model = RFDETRBase(**kwargs)
        return self._model

    def detect(self, image) -> list[Box]:  # pragma: no cover - needs rfdetr
        import numpy as np

        model = self._load()
        arr = np.asarray(image)
        h, w = arr.shape[:2]
        detections = model.predict(arr, threshold=self.threshold)

        # rfdetr returns a supervision.Detections: .xyxy (N,4) abs pixels,
        # .confidence (N,). Normalize to Box. Single class -> interactable.
        xyxy = getattr(detections, "xyxy", None)
        conf = getattr(detections, "confidence", None)
        if xyxy is None:
            return []
        boxes: list[Box] = []
        for i in range(len(xyxy)):
            x1, y1, x2, y2 = (float(v) for v in xyxy[i])
            score = float(conf[i]) if conf is not None else 1.0
            boxes.append(
                Box(
                    x=x1 / w,
                    y=y1 / h,
                    w=max(0.0, x2 - x1) / w,
                    h=max(0.0, y2 - y1) / h,
                    score=score,
                    interactable=True,
                    label="interactable",
                )
            )
        return boxes
