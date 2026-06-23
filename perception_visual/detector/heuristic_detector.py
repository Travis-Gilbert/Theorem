"""License-clean, dependency-light fallback detector.

`HeuristicDetector` needs only numpy. It proposes interactable regions from edge
density on a coarse grid so the pipeline, the FastAPI endpoint, and the eval
harness all run end to end with no model download. It is explicitly NOT an
accuracy path: production perception uses `RfDetrDetector`. It exists so the
wiring is testable offline and so the eval harness can print a baseline without
weights. Being pure numpy, it carries no third-party license obligations.
"""
from __future__ import annotations

import numpy as np

from .base import Box


class HeuristicDetector:
    name = "heuristic-grid"

    def __init__(self, grid: int = 16, keep_percentile: float = 80.0, max_boxes: int = 64):
        self.grid = int(grid)
        self.keep_percentile = float(keep_percentile)
        self.max_boxes = int(max_boxes)

    def detect(self, image) -> list[Box]:
        arr = np.asarray(image)
        if arr.ndim == 2:
            arr = arr[:, :, None]
        h, w = arr.shape[:2]
        if h < 2 or w < 2:
            return []
        chans = arr.shape[2]
        gray = arr[:, :, :3].mean(axis=2) if chans >= 3 else arr[:, :, 0].astype(float)

        # Cheap gradient magnitude (no scipy/cv2): abs first-differences in x, y.
        gx = np.zeros_like(gray, dtype=float)
        gy = np.zeros_like(gray, dtype=float)
        gx[:, 1:] = np.abs(np.diff(gray, axis=1))
        gy[1:, :] = np.abs(np.diff(gray, axis=0))
        edges = gx + gy

        g = self.grid
        cell_h = max(1, h // g)
        cell_w = max(1, w // g)
        cells = []
        for gy_i in range(g):
            for gx_i in range(g):
                y0, x0 = gy_i * cell_h, gx_i * cell_w
                if y0 >= h or x0 >= w:
                    continue
                y1 = h if gy_i == g - 1 else min(h, y0 + cell_h)
                x1 = w if gx_i == g - 1 else min(w, x0 + cell_w)
                cells.append((float(edges[y0:y1, x0:x1].mean()), x0, y0, x1, y1))

        if not cells:
            return []
        densities = np.array([c[0] for c in cells], dtype=float)
        peak = float(densities.max())
        if peak <= 0.0:
            return []
        thresh = float(np.percentile(densities, self.keep_percentile))

        boxes: list[Box] = []
        for density, x0, y0, x1, y1 in cells:
            if density < thresh or density <= 0.0:
                continue
            boxes.append(
                Box(
                    x=x0 / w,
                    y=y0 / h,
                    w=(x1 - x0) / w,
                    h=(y1 - y0) / h,
                    score=min(1.0, density / peak),
                    interactable=True,
                    label="region",
                )
            )
        boxes.sort(key=lambda b: b.score, reverse=True)
        return boxes[: self.max_boxes]
