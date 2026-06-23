from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.expression.registry import get_expression_registry


class ExpressionEngineTests(SimpleTestCase):
    def test_structured_result_renders_as_brief_and_report(self):
        result = {'engine': 'solver', 'status': 'sat', 'counterexample': {'violations': [{'constraint_id': 'c1'}]}}
        registry = get_expression_registry()

        brief = registry.render('deterministic_brief', result)
        report = registry.render('structured_report', result)

        self.assertEqual(brief.artifact_type, 'brief')
        self.assertEqual(report.artifact_type, 'report')
        self.assertIn('Counterexample', {section['title'] for section in report.payload['sections']})

    def test_scene_package_shape_for_solver_output(self):
        result = {
            'title': 'Solver result',
            'formula_hash': 'abc1234567890',
            'counterexample': {'violations': [{'constraint_id': 'privacy', 'label': 'Private export'}]},
        }

        scene = get_expression_registry().render('scene_package', result)

        self.assertEqual(scene.artifact_type, 'scene_package')
        self.assertEqual(scene.payload['manifest']['surface'], 'dashboard')
        self.assertEqual(scene.payload['datasets'][0]['shape'], 'evidence_stack')

