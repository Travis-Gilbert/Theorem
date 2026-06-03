#!/usr/bin/env python3
# Python smoke for the theorem-harness UniFFI binding (third language).
#
# Proves the round-trip Python -> generated bindings -> Rust SDK -> RedCore ->
# Python, the same lifecycle the Node and Swift smokes exercise, against the SAME
# Rust core. The Python module is generated from the same crate as the Swift one
# (uniffi is multi-language from one annotated facade). Run:
#   cargo build --release --lib
#   cargo run --release --bin uniffi-bindgen -- generate \
#     --library target/release/libtheorem_harness_swift.dylib --language python \
#     --out-dir generated-python
#   cp target/release/libtheorem_harness_swift.dylib generated-python/   # next to the .py
#   python3 smoke.py

import json
import os
import sys
import tempfile
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "generated-python"))

from theorem_harness_swift import Harness  # noqa: E402


def kinds(events_json: str) -> list[str]:
    return [event.get("kind", "?") for event in json.loads(events_json)]


def main() -> int:
    data_dir = os.path.join(tempfile.gettempdir(), "theorem-harness-py-" + uuid.uuid4().hex)
    os.makedirs(data_dir, exist_ok=True)
    print("data dir:", data_dir)

    harness = Harness(data_dir)

    run_id = harness.start_run("demo from python", "py-smoke", "k-create")
    print("started run:", run_id)

    after_start = kinds(harness.events_json(run_id))
    print("after start:", after_start)

    harness.cancel(run_id, "stopping from python", "k-cancel")
    after_cancel = kinds(harness.events_json(run_id))
    print("after cancel:", after_cancel)

    status = harness.run_status(run_id)
    print("status:", status)

    harness.remember(
        "py-smoke", "belief", "binding is durable", "The Python binding persists to RedCore."
    )
    recalled = json.loads(harness.recall("py-smoke", "binding", 10))
    print("recalled:", len(recalled))

    ok = (
        after_start == ["Created"]
        and after_cancel == ["Created", "Cancelled"]
        and status == "cancelled"
        and len(recalled) >= 1
    )
    print("SMOKE PASS" if ok else "SMOKE FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
