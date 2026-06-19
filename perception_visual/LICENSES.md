# perception_visual -- license record

Hard gate for shipping inside the commercial harness: **no AGPL, no GPL, no
Ultralytics.** OmniParser's interactable-region detector is a fine-tuned
Ultralytics YOLOv8 (AGPL-3.0); this package deletes that surface and replaces it
with Apache-2.0 RF-DETR. OmniParser's MIT pipeline and MIT Florence-2 captioner
are kept; OCR uses Apache-2.0 EasyOCR.

## Per-component licenses (loaded model weights and libraries)

| Component | Role | License (SPDX) | Source |
|---|---|---|---|
| RF-DETR core (`rfdetr`, Nano-Large incl. `rf-detr-base`) -- code **and** COCO-pretrained weights | interactable-region detector (production) | **Apache-2.0** | https://github.com/roboflow/rf-detr , https://github.com/roboflow/rf-detr/blob/develop/LICENSE |
| Florence-2-base | icon captioner | **MIT** | https://huggingface.co/microsoft/Florence-2-base |
| EasyOCR -- code and detection/recognition weights | OCR | **Apache-2.0** | https://github.com/JaidedAI/EasyOCR |
| Pillow | image decode / overlay render | **MIT-CMU (HPND)** | https://github.com/python-pillow/Pillow |
| NumPy | core arrays, HeuristicDetector | **BSD-3-Clause** | https://github.com/numpy/numpy |
| FastAPI | `/parse` serving | **MIT** | https://github.com/fastapi/fastapi |
| Pydantic | request/response schema | **MIT** | https://github.com/pydantic/pydantic |
| Uvicorn | ASGI server (run only) | **BSD-3-Clause** | https://github.com/encode/uvicorn |
| Transformers | Florence-2 runtime | **Apache-2.0** | https://github.com/huggingface/transformers |
| PyTorch | Florence-2 / RF-DETR runtime | **BSD-3-Clause** | https://github.com/pytorch/pytorch |
| Playwright (Python) | dataset rendering | **Apache-2.0** | https://github.com/microsoft/playwright-python |

## Explicitly excluded

| Excluded | License | Why |
|---|---|---|
| Ultralytics YOLOv8 (OmniParser's `icon_detect` weights) | **AGPL-3.0 / paid Enterprise** | AGPL network trigger covers served weights; cannot ship in a closed harness. Replaced by RF-DETR. There is no `ultralytics` import anywhere in this package (enforced by `tests/test_pipeline.py::test_no_ultralytics_import_in_package`). |
| RF-DETR+ XL / 2XL (`rf-detr_plus`) checkpoints | **PML-1.0** (Platform Model License 1.0, custom non-OSI) | Not AGPL/GPL, but a custom license with extra terms. **Do not pin the detector to XL/2XL.** Use only the Apache-2.0 core `rfdetr` models (Nano-Large, incl. `rf-detr-base`). |

## Verification status

- Licenses above were verified on 2026-06-19 against the projects' published
  GitHub / Hugging Face license metadata (RF-DETR core code + COCO weights:
  Apache-2.0; Florence-2-base: MIT; EasyOCR code + weights: Apache-2.0).
- **Artifact-level verification is still required at deploy time.** No model
  weights were downloaded in this build. Before production, pull each weight and
  confirm the bundled `LICENSE` of the *actual artifact* matches the entry above
  (in particular, confirm the RF-DETR checkpoint you pull is a core Apache-2.0
  model and not an `rf-detr_plus` PML-1.0 model).
- The "no Ultralytics import" gate is a unit test, so it fails CI if anyone
  reintroduces the AGPL detector.
