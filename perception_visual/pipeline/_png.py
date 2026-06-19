"""Minimal dependency-free PNG writer (stdlib zlib only).

Used to render the annotated overlay when Pillow is not installed, so `parse`
always returns real PNG bytes. With Pillow present the pipeline draws a richer
overlay (numeric labels); without it, this writes box borders only.
"""
from __future__ import annotations

import struct
import zlib

import numpy as np


def encode_png(rgb: np.ndarray) -> bytes:
    arr = np.ascontiguousarray(rgb[:, :, :3].astype(np.uint8))
    h, w = arr.shape[:2]
    raw = bytearray()
    for y in range(h):
        raw.append(0)  # PNG filter type 0 (None) per scanline
        raw.extend(arr[y].tobytes())

    def chunk(typ: bytes, data: bytes) -> bytes:
        c = typ + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    out = b"\x89PNG\r\n\x1a\n"
    out += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))  # 8-bit RGB
    out += chunk(b"IDAT", zlib.compress(bytes(raw), 6))
    out += chunk(b"IEND", b"")
    return out
