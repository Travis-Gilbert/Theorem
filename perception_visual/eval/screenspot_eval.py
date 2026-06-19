"""Baseline element-localization (grounding) eval, ScreenSpot-style.

Reports grounding accuracy: a ground-truth element counts as localized if its
center falls inside any predicted interactable box. Runs the chosen detector
over a split's images and prints one baseline number. The bundled `heldout`
split is small and synthetic (planted high-contrast regions) so the harness
runs offline with the numpy-only HeuristicDetector; point `--data` at a real
ScreenSpot-style split for a real number.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from ..detector.base import Box

_HERE = Path(__file__).resolve().parent
_DEFAULT_SPLITS = {"heldout": _HERE / "fixtures" / "heldout"}


def center_in_box(center: tuple[float, float], box: Box) -> bool:
    cx, cy = center
    return box.x <= cx <= box.x2 and box.y <= cy <= box.y2


def grounding_hits(pred_boxes: list[Box], gt_centers: list[tuple[float, float]]) -> int:
    """Count GT centers covered by at least one predicted box (all normalized)."""
    return sum(1 for c in gt_centers if any(center_in_box(c, b) for b in pred_boxes))


def grounding_accuracy(per_image: list[tuple[list[Box], list[tuple[float, float]]]]) -> dict:
    """Aggregate accuracy over images. Returns {hits, total, accuracy}."""
    hits = total = 0
    for pred_boxes, gt_centers in per_image:
        total += len(gt_centers)
        hits += grounding_hits(pred_boxes, gt_centers)
    return {"hits": hits, "total": total, "accuracy": (hits / total) if total else 0.0}


def _gt_centers_from_coco(coco: dict) -> dict[int, list[tuple[float, float]]]:
    """Map image_id -> list of normalized GT box centers."""
    size = {img["id"]: (img["width"], img["height"]) for img in coco["images"]}
    centers: dict[int, list[tuple[float, float]]] = {i: [] for i in size}
    for ann in coco["annotations"]:
        w, h = size[ann["image_id"]]
        x, y, bw, bh = ann["bbox"]
        centers[ann["image_id"]].append(((x + bw / 2) / w, (y + bh / 2) / h))
    return centers


def _load_image(path: Path):
    from PIL import Image  # eval reads on-disk PNGs

    import numpy as np

    return np.asarray(Image.open(path).convert("RGB"))


def evaluate(split_dir: str | Path, detector=None) -> dict:
    """Run the detector over a split and return the grounding-accuracy report."""
    split_dir = Path(split_dir)
    coco = json.loads((split_dir / "gt.json").read_text())
    centers_by_id = _gt_centers_from_coco(coco)
    images_dir = split_dir / "images"

    if detector is None:
        from ..detector.heuristic_detector import HeuristicDetector

        detector = HeuristicDetector()

    per_image = []
    for img in coco["images"]:
        arr = _load_image(images_dir / img["file_name"])
        pred_boxes = detector.detect(arr)
        per_image.append((pred_boxes, centers_by_id[img["id"]]))

    report = grounding_accuracy(per_image)
    report["detector"] = getattr(detector, "name", type(detector).__name__)
    report["images"] = len(coco["images"])
    return report


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Baseline grounding eval for perception_visual.")
    ap.add_argument("--split", default="heldout", help="named split (bundled: heldout)")
    ap.add_argument("--data", help="explicit split dir (overrides --split); needs gt.json + images/")
    ap.add_argument("--detector", choices=["heuristic", "rfdetr"], default="heuristic")
    args = ap.parse_args(argv)

    split_dir = Path(args.data) if args.data else _DEFAULT_SPLITS.get(args.split)
    if split_dir is None or not Path(split_dir).exists():
        ap.error(f"unknown split '{args.split}'; pass --data <dir>")

    detector = None
    if args.detector == "rfdetr":
        from ..detector.rfdetr_detector import RfDetrDetector

        detector = RfDetrDetector()

    report = evaluate(split_dir, detector=detector)
    synthetic = args.data is None and args.split == "heldout"
    print(
        f"split={args.split} detector={report['detector']} "
        f"images={report['images']} elements={report['total']} "
        f"grounding_accuracy={report['accuracy'] * 100:.1f}% "
        f"({report['hits']}/{report['total']})"
        + ("  [synthetic bundled split]" if synthetic else "")
    )
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
