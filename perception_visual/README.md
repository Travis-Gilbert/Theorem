# perception_visual

License-clean visual GUI perception: a screenshot in, labeled interactable
elements (plus optional captions) out. This is OmniParser's pipeline with the
**AGPL Ultralytics YOLOv8 detector removed** and replaced by Apache-2.0 RF-DETR,
so it can ship inside the commercial harness. OmniParser's MIT Set-of-Mark
pipeline and MIT Florence-2 captioner are kept; OCR uses Apache-2.0 EasyOCR.

It is the **fallback perceiver** for surfaces with no accessibility/DOM tree
(desktop apps, canvas content). On web surfaces the harness prefers the
accessibility tree and addresses real elements; this is the no-tree path.

## Location

This package lives at the repo-root path `perception_visual/`, the candidate
named in the build handoff. It is self-contained (own `pyproject.toml`, scoped
to this package only) and is destined for its own RunPod serving deployment. If
the team prefers it under `apps/`, it relocates cleanly -- every import is
package-internal.

## Architecture: two modes, graceful degradation

The package core (detector protocol, SOM box geometry, COCO assembly, eval
metric, annotated-image raster) depends on **numpy + stdlib only** and is fully
tested without any model installed. Every heavy stack is imported *lazily* by
the function that needs it:

- detector: `RfDetrDetector` (Apache-2.0 RF-DETR) in production; the numpy-only
  `HeuristicDetector` offline so `parse`, `/parse`, and the eval harness all run
  with no model download.
- OCR: `easyocr` (lazy; degrades to no text).
- caption: Florence-2 via `transformers` (lazy; degrades to empty captions).
- dataset build: `playwright` (lazy).

```text
perception_visual/
  detector/   base.py (Box + Detector protocol) | rfdetr_detector.py | heuristic_detector.py
  pipeline/   som.py (parse + SOM merge) | ocr.py | caption.py | _png.py (stdlib PNG fallback)
  data/       build_dataset.py (accessibility-tree -> COCO)
  train/      finetune.py (RF-DETR fine-tune)
  eval/       screenspot_eval.py + fixtures/heldout (synthetic split)
  serve/      app.py (FastAPI POST /parse)
  tests/      test_pipeline.py
  LICENSES.md
```

## Install

```bash
# run from the repo root, where pyproject.toml (scoped to this package) lives
python -m venv .venv && . .venv/bin/activate
pip install -e ".[dev]"          # core + pytest (numpy, pillow)
# production / serving extras, installed as needed:
pip install -e ".[detect]"       # RF-DETR (Apache-2.0)
pip install -e ".[caption]"      # Florence-2 (transformers, torch)
pip install -e ".[ocr]"          # EasyOCR
pip install -e ".[serve]"        # FastAPI + uvicorn + pillow
pip install -e ".[data]"         # Playwright (then: playwright install chromium)
```

## Run

```bash
# tests (numpy + stdlib core; pillow/fastapi tests run when those are installed)
python -m pytest perception_visual/tests/ -v

# baseline grounding eval (bundled synthetic split, offline HeuristicDetector)
python -m perception_visual.eval.screenspot_eval --split heldout
#   point at a real ScreenSpot-style split:  --data <dir>  (needs gt.json + images/)
#   use the production detector:              --detector rfdetr

# build a COCO dataset from rendered pages (needs Playwright + chromium)
python -m perception_visual.data.build_dataset --urls https://example.com --out data_out

# fine-tune RF-DETR (real run needs rfdetr + GPU; --dry-run validates offline)
python -m perception_visual.train.finetune --dataset <coco_dir> --output ckpt --dry-run

# serve POST /parse for the Rust theorem-browser-agent perceiver
uvicorn perception_visual.serve.app:app --port 8080
```

Note: the eval / dataset / train modules use package-relative imports, so run
them with `python -m perception_visual.<module>` (not `python path/to/file.py`).

## Baseline

`screenspot_eval --split heldout` reports **grounding accuracy 94.4% (17/18)**
on the bundled split. That split is **synthetic** (planted high-contrast regions
on dark backgrounds) and is run with the offline `HeuristicDetector` -- it proves
the eval harness end to end, not real-world accuracy. The one miss is a large
interior panel (low edge density). Iterating the RF-DETR detector to real
ScreenSpot parity on a labeled set is the named follow-up.

## `POST /parse`

Request: `{"image_base64": "<png/jpeg bytes b64>", "media_type": "image/png",
"use_ocr": true, "caption": false, "include_annotated": true}`.
Response: `{"image_size", "count", "elements": [{id, interactable, source,
content, score, box{x,y,w,h}, box_pixels{...}}], "annotated_image_base64"}`.

## License

Apache-2.0. All loaded model weights and libraries are permissive (Apache-2.0 or
MIT) and verified non-AGPL/GPL; see [LICENSES.md](LICENSES.md). The Ultralytics
AGPL detector is excluded and the exclusion is enforced by a unit test.

## What is verified here vs. what needs the model stack

Verified offline (this build, Python 3.14 + numpy/pillow/fastapi):
`parse()` end to end, SOM merge/geometry, COCO assembly, the eval harness +
baseline number, `POST /parse` (TestClient), `finetune --dry-run`, and the
no-Ultralytics license gate -- 19 tests green.

Needs the model stack / GPU / network (not run here): real RF-DETR detection +
fine-tune checkpoint, Florence-2 captions, EasyOCR text, Playwright dataset
capture, and a real ScreenSpot accuracy number.

## Follow-ups

- Iterate the RF-DETR detector to ScreenSpot parity on a larger labeled set.
- Wire `theorem-browser-agent`'s perceiver to `POST /parse` so the model grounds
  on labeled elements for no-DOM surfaces.
