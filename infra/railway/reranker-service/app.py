import os
import threading
from typing import Optional

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field
from sentence_transformers import CrossEncoder


DEFAULT_MODEL_ID = "Alibaba-NLP/gte-reranker-modernbert-base"
MAX_TEXTS = int(os.getenv("MAX_TEXTS", "128"))
DIVERSITY_WEIGHT = float(os.getenv("DIVERSITY_WEIGHT", "0.0"))
TRUST_REMOTE_CODE = os.getenv("TRUST_REMOTE_CODE", "false").lower() in {
    "1",
    "true",
    "yes",
    "on",
}

app = FastAPI(title="Theorem reranker service", version="1.0")
_models: dict[str, CrossEncoder] = {}
_lock = threading.Lock()


class RerankRequest(BaseModel):
    query: str
    text: Optional[str] = None
    texts: list[str] = Field(default_factory=list)
    model: Optional[str] = None
    top_k: Optional[int] = None


def model_id(requested: Optional[str]) -> str:
    value = (requested or os.getenv("MODEL_ID") or DEFAULT_MODEL_ID).strip()
    return value or DEFAULT_MODEL_ID


def get_model(name: str) -> CrossEncoder:
    with _lock:
        if name not in _models:
            _models[name] = CrossEncoder(name, trust_remote_code=TRUST_REMOTE_CODE)
        return _models[name]


def request_texts(request: RerankRequest) -> list[str]:
    texts = list(request.texts)
    if request.text is not None:
        texts.insert(0, request.text)
    if not texts:
        raise HTTPException(status_code=400, detail="text or texts is required")
    if len(texts) > MAX_TEXTS:
        raise HTTPException(status_code=400, detail=f"too many texts: max {MAX_TEXTS}")
    return texts


def score_texts(request: RerankRequest) -> tuple[str, list[str], list[float]]:
    name = model_id(request.model)
    texts = request_texts(request)
    pairs = [(request.query, text) for text in texts]
    scores = get_model(name).predict(pairs, show_progress_bar=False)
    return name, texts, [float(score) for score in scores]


def token_set(text: str) -> set[str]:
    return {part.lower() for part in text.split() if len(part) > 2}


def jaccard(left: set[str], right: set[str]) -> float:
    if not left or not right:
        return 0.0
    return len(left & right) / len(left | right)


def diversified_order(texts: list[str], scores: list[float]) -> list[int]:
    remaining = set(range(len(texts)))
    selected: list[int] = []
    token_sets = [token_set(text) for text in texts]
    while remaining:
        def adjusted(index: int) -> float:
            redundancy = max(
                (jaccard(token_sets[index], token_sets[chosen]) for chosen in selected),
                default=0.0,
            )
            return scores[index] - DIVERSITY_WEIGHT * redundancy

        winner = max(remaining, key=lambda index: (adjusted(index), scores[index], -index))
        selected.append(winner)
        remaining.remove(winner)
    return selected


@app.get("/healthz")
def healthz() -> dict:
    return {
        "ok": True,
        "default_model": model_id(None),
        "loaded_models": sorted(_models.keys()),
        "diversity_weight": DIVERSITY_WEIGHT,
    }


@app.post("/score")
def score(request: RerankRequest) -> dict:
    name, texts, scores = score_texts(request)
    return {
        "model": name,
        "score": scores[0],
        "scores": scores,
        "count": len(texts),
    }


@app.post("/rerank")
def rerank(request: RerankRequest) -> list[dict]:
    name, texts, scores = score_texts(request)
    order = diversified_order(texts, scores) if DIVERSITY_WEIGHT > 0.0 else sorted(
        range(len(texts)),
        key=lambda index: (scores[index], -index),
        reverse=True,
    )
    if request.top_k is not None:
        order = order[: max(0, request.top_k)]
    return [
        {
            "index": index,
            "score": scores[index],
            "model": name,
            "text": texts[index],
        }
        for index in order
    ]
