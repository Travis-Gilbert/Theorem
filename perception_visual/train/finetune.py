"""RF-DETR fine-tune entrypoint.

Fine-tunes the Apache-2.0 RF-DETR detector on a COCO dataset produced by
`perception_visual.data.build_dataset`. The real train needs `rfdetr` + a GPU
and produces a checkpoint; `--dry-run` validates the dataset and prints the
train plan offline (no model stack), so the entrypoint and data wiring are
provable without weights.
"""
from __future__ import annotations

import argparse
import json
import logging
from pathlib import Path

logger = logging.getLogger(__name__)


def validate_coco_dataset(dataset_dir: str | Path) -> dict:
    """Validate a COCO dataset directory and return summary stats.

    Checks coco.json parses, every image file exists, and counts boxes. Pure
    (no model deps) so it is unit-testable and powers `--dry-run`.
    """
    dataset_dir = Path(dataset_dir)
    coco_path = dataset_dir / "coco.json"
    if not coco_path.exists():
        raise FileNotFoundError(f"no coco.json in {dataset_dir}")
    coco = json.loads(coco_path.read_text())

    images = coco.get("images", [])
    annotations = coco.get("annotations", [])
    categories = coco.get("categories", [])
    images_dir = dataset_dir / "images"

    missing = []
    for img in images:
        fp = images_dir / img["file_name"]
        if not fp.exists():
            missing.append(img["file_name"])

    return {
        "images": len(images),
        "annotations": len(annotations),
        "categories": [c["name"] for c in categories],
        "missing_files": missing,
        "boxes_per_image": (len(annotations) / len(images)) if images else 0.0,
    }


def finetune(
    dataset_dir: str | Path,
    output_dir: str | Path,
    *,
    epochs: int = 10,
    batch_size: int = 4,
    checkpoint: str | None = None,
) -> str:  # pragma: no cover - needs rfdetr + GPU
    """Fine-tune RF-DETR; returns the output checkpoint directory.

    Requires the Apache-2.0 `rfdetr` package, imported lazily.
    """
    try:
        from rfdetr import RFDETRBase  # type: ignore
    except Exception as exc:
        raise ImportError(
            "finetune requires the Apache-2.0 'rfdetr' package (pip install rfdetr). "
            "Use --dry-run to validate the dataset offline."
        ) from exc

    stats = validate_coco_dataset(dataset_dir)
    if stats["missing_files"]:
        raise FileNotFoundError(f"{len(stats['missing_files'])} image files missing")

    model = RFDETRBase(**({"pretrain_weights": checkpoint} if checkpoint else {}))
    model.train(
        dataset_dir=str(dataset_dir),
        epochs=epochs,
        batch_size=batch_size,
        output_dir=str(output_dir),
    )
    return str(output_dir)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Fine-tune RF-DETR on an interactable-element COCO dataset.")
    ap.add_argument("--dataset", required=True, help="COCO dataset directory (with coco.json + images/)")
    ap.add_argument("--output", default="checkpoints/rf-detr-interactable", help="checkpoint output dir")
    ap.add_argument("--epochs", type=int, default=10)
    ap.add_argument("--batch-size", type=int, default=4)
    ap.add_argument("--checkpoint", default=None, help="warm-start weights")
    ap.add_argument("--dry-run", action="store_true", help="validate dataset + print plan, no training")
    args = ap.parse_args(argv)

    stats = validate_coco_dataset(args.dataset)
    print("dataset:", json.dumps(stats, indent=2))
    if stats["missing_files"]:
        print(f"ERROR: {len(stats['missing_files'])} referenced image files missing")
        return 1

    if args.dry_run:
        plan = {
            "detector": "rf-detr (Apache-2.0)",
            "dataset_dir": args.dataset,
            "output_dir": args.output,
            "epochs": args.epochs,
            "batch_size": args.batch_size,
            "warm_start": args.checkpoint,
        }
        out = Path(args.output)
        out.mkdir(parents=True, exist_ok=True)
        (out / "train_plan.json").write_text(json.dumps(plan, indent=2))
        print("dry-run OK; plan written to", out / "train_plan.json")
        print("real training needs `rfdetr` + a GPU and produces the checkpoint here.")
        return 0

    ckpt = finetune(
        args.dataset,
        args.output,
        epochs=args.epochs,
        batch_size=args.batch_size,
        checkpoint=args.checkpoint,
    )
    print("checkpoint written to", ckpt)
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
