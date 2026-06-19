"""Generate the synthetic `heldout` eval split (deterministic).

Writes images/*.png + gt.json under this directory. The images plant bright
high-contrast rectangles ("icons") on a dark background at known locations; the
GT boxes are those rectangles. This lets the offline HeuristicDetector be scored
end to end without any real ScreenSpot data. It is explicitly synthetic; point
`screenspot_eval --data <dir>` at a real split for a real number.

Run: python perception_visual/eval/fixtures/_generate_heldout.py
"""
from __future__ import annotations

import json
from pathlib import Path

import numpy as np

from perception_visual.pipeline._png import encode_png

HERE = Path(__file__).resolve().parent
HELDOUT = HERE / "heldout"
IMAGES = HELDOUT / "images"

# (file_name, [ (x, y, w, h) abs-pixel GT rectangles ])
LAYOUTS = [
    ("page_00.png", [(40, 40, 14, 14), (96, 60, 14, 14), (160, 48, 16, 16), (208, 120, 14, 14)]),
    ("page_01.png", [(48, 32, 16, 16), (120, 96, 14, 14), (190, 160, 16, 16), (64, 200, 14, 14)]),
    ("page_02.png", [(32, 32, 14, 14), (96, 32, 14, 14), (160, 32, 14, 14), (224, 32, 14, 14)]),
    ("page_03.png", [(50, 50, 16, 16), (150, 150, 16, 16), (20, 120, 80, 48)]),  # last = large panel
    ("page_04.png", [(72, 64, 14, 14), (140, 110, 14, 14), (200, 200, 16, 16)]),
]
SIZE = 256
BG = 30
FG = 225


def main() -> None:
    IMAGES.mkdir(parents=True, exist_ok=True)
    images, annotations = [], []
    ann_id = 1
    for img_id, (name, rects) in enumerate(LAYOUTS, start=1):
        canvas = np.full((SIZE, SIZE, 3), BG, dtype=np.uint8)
        for x, y, w, h in rects:
            canvas[y : y + h, x : x + w] = FG
        (IMAGES / name).write_bytes(encode_png(canvas))
        images.append({"id": img_id, "file_name": name, "width": SIZE, "height": SIZE})
        for x, y, w, h in rects:
            annotations.append(
                {
                    "id": ann_id,
                    "image_id": img_id,
                    "category_id": 1,
                    "bbox": [x, y, w, h],
                    "area": w * h,
                    "iscrowd": 0,
                }
            )
            ann_id += 1
    coco = {
        "images": images,
        "annotations": annotations,
        "categories": [{"id": 1, "name": "interactable"}],
    }
    (HELDOUT / "gt.json").write_text(json.dumps(coco, indent=2))
    print(f"wrote {len(images)} images, {len(annotations)} GT boxes -> {HELDOUT}")


if __name__ == "__main__":
    main()
