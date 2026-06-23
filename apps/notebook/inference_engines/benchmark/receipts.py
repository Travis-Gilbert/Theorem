"""Content-address a produced engine receipt for the ledger receipt_hash field.

Binds the ledger to the real DatalogReceipt (and any to_dict-able receipt) via
the existing stable_hash. This is the seam between CO-0 (an affordance produces
a receipt) and CO-1 (the ledger references it by hash). Generic by design so the
probabilistic receipt lands here too without a second implementation.
"""

from __future__ import annotations

from typing import Any

from ..common import stable_hash


def receipt_hash_for(receipt: Any) -> str:
    if receipt is None:
        return ''
    if hasattr(receipt, 'to_dict'):
        return stable_hash(receipt.to_dict())
    if isinstance(receipt, dict):
        return stable_hash(receipt)
    raise TypeError('receipt_hash_for expects a to_dict-able receipt or a dict')
