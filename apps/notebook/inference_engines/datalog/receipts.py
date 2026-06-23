"""Receipt rendering helpers for symbolic derivation results."""

from __future__ import annotations

from .contracts import DatalogReceipt


def receipt_summary(receipt: DatalogReceipt) -> dict:
    payload = receipt.to_dict()
    return {
        'engine': payload['engine'],
        'fact_pack_hash': payload['fact_pack_hash'],
        'derived_count': payload['derived_count'],
        'rule_ids': payload['rule_ids'],
        'warnings': payload['warnings'],
        'writeback_policy': payload['writeback_policy'],
    }

