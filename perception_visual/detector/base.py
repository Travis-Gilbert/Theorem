"""Detector contract for visual GUI perception.

`Box` is the normalized detection record every detector returns; `Detector` is
the protocol the SOM pipeline calls. Both are stdlib-only so the pipeline,
tests, and eval run without any model stack installed. Keeping the pipeline
behind this protocol is exactly what lets us swap OmniParser's AGPL Ultralytics
detector for an Apache-2.0 one without touching the pipeline.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol, runtime_checkable


@dataclass(frozen=True)
class Box:
    """An axis-aligned detection in normalized [0, 1] image coordinates.

    `x`, `y` are the top-left corner; `w`, `h` the width/height, each divided by
    the image width/height so a box is resolution-independent.
    """

    x: float
    y: float
    w: float
    h: float
    score: float
    interactable: bool = True
    label: str = ""

    @property
    def x2(self) -> float:
        return self.x + self.w

    @property
    def y2(self) -> float:
        return self.y + self.h

    @property
    def area(self) -> float:
        return max(0.0, self.w) * max(0.0, self.h)

    def xyxy(self) -> tuple[float, float, float, float]:
        return (self.x, self.y, self.x2, self.y2)


def iou(a: Box, b: Box) -> float:
    """Intersection-over-union of two normalized boxes."""
    ix1, iy1 = max(a.x, b.x), max(a.y, b.y)
    ix2, iy2 = min(a.x2, b.x2), min(a.y2, b.y2)
    inter = max(0.0, ix2 - ix1) * max(0.0, iy2 - iy1)
    union = a.area + b.area - inter
    return inter / union if union > 0 else 0.0


def intersection_over_smaller(a: Box, b: Box) -> float:
    """Fraction of the smaller box covered by the overlap.

    OmniParser uses this (not plain IoU) to drop an icon box that sits almost
    entirely inside a text box (or vice versa), where IoU stays low because the
    two boxes differ a lot in size.
    """
    ix1, iy1 = max(a.x, b.x), max(a.y, b.y)
    ix2, iy2 = min(a.x2, b.x2), min(a.y2, b.y2)
    inter = max(0.0, ix2 - ix1) * max(0.0, iy2 - iy1)
    smaller = min(a.area, b.area)
    return inter / smaller if smaller > 0 else 0.0


@runtime_checkable
class Detector(Protocol):
    """Anything that turns an image into interactable-region boxes.

    `image` is an HxWx3 uint8 RGB numpy array. Implementations must be swappable
    behind this protocol so the pipeline never imports a specific model stack.
    """

    name: str

    def detect(self, image) -> list[Box]:  # noqa: ANN001 - numpy ndarray
        ...
