"""Scene OS package compiler for inference receipts."""

from __future__ import annotations

from apps.notebook.scene_os.contracts import (
    SceneAtom,
    SceneDataset,
    SceneManifest,
    ScenePackage,
    ScenePanel,
    SceneTrace,
)

from .contracts import ExpressionInput, ExpressionResult


def compile_scene(input_payload: ExpressionInput) -> ExpressionResult:
    result = input_payload.result
    rows = _rows_for_result(result)
    atoms = tuple(
        SceneAtom(
            id=f'inference-{index}',
            label=str(row.get('label') or row.get('relation') or row.get('constraint_id') or f'Item {index + 1}'),
            kind=str(row.get('kind') or 'inference'),
            role=str(row.get('role') or 'evidence'),
            data=row,
            provenance={'engine': result.get('engine') or result.get('provider') or ''},
        )
        for index, row in enumerate(rows)
    )
    manifest = SceneManifest(
        scene_id=f'inference-{(result.get("receipt_hash") or result.get("formula_hash") or "scene")[:16]}',
        title=str(result.get('title') or 'Inference Receipt'),
        summary=str(result.get('summary') or result.get('reason') or 'Structured inference receipt.'),
        surface='dashboard',
        renderer='evidence_board',
        atoms=atoms,
        panels=(ScenePanel(
            id='receipt-panel',
            renderer='evidence_board',
            title='Receipt',
            atom_ids=tuple(atom.id for atom in atoms),
            data_shape='evidence_stack',
        ),),
        data_shapes=('evidence_stack',),
        metadata={'inference_result': True},
    )
    package = ScenePackage(
        manifest=manifest,
        datasets=(SceneDataset(id='receipt-rows', shape='evidence_stack', rows=tuple(rows)),),
        traces=(SceneTrace(id='expression-scene', step='compile', message='Compiled inference receipt into Scene OS package.'),),
        provenance={'compiler': 'inference_engines.expression.scene_compiler'},
    )
    return ExpressionResult(
        engine_id='scene_package_compiler',
        artifact_type='scene_package',
        payload=package.to_dict(),
    )


def _rows_for_result(result: dict) -> list[dict]:
    if isinstance(result.get('derived_facts'), list):
        return [dict(item) for item in result['derived_facts']]
    if isinstance(result.get('rewrite_trace'), list):
        return [dict(item) for item in result['rewrite_trace']]
    if isinstance(result.get('counterexample'), dict):
        violations = result['counterexample'].get('violations')
        if isinstance(violations, list):
            return [dict(item) for item in violations]
        return [dict(result['counterexample'])]
    if isinstance(result.get('outcomes'), list):
        return [dict(item) for item in result['outcomes']]
    return [dict(result)]

