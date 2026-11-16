from __future__ import annotations

import json
import unittest
import urllib.request

from moli_cdp_smoke.fixture import FixtureServer
from moli_cdp_smoke.groups.web_security_upstream import CONTRACTS, SOURCES, compare_upstream
from moli_cdp_smoke.runner import DEFAULT_GROUP_NAMES, OPTIONAL_PROCESS_GROUPS, resolve_group_selection


class ChromiumWebSecurityPortsTests(unittest.TestCase):
    def test_every_oracle_case_has_an_upstream_source_or_explicit_additional_guard(self):
        sources = json.loads(SOURCES.read_text())
        contracts = json.loads(CONTRACTS.read_text())
        self.assertEqual(sources['chromium']['oracle_version'], '156.0.8078.73')
        self.assertEqual(len(sources['chromium']['revision']), 40)
        mapped = {case for entry in sources['entries'] for case in entry['cases']}
        self.assertEqual(mapped, set(contracts['default']))
        self.assertEqual(mapped, set(contracts['disabled']))
        for entry in sources['entries']:
            with self.subTest(path=entry['path'], tests=entry['tests']):
                self.assertEqual(len(entry['sha256']), 64)
                self.assertTrue(entry['tests'])
                self.assertTrue(entry['notes'])
                self.assertIn(entry['disposition'], {
                    'port', 'observable_guard', 'additional_guard', 'different_configuration',
                    'browser_internal', 'chrome_platform', 'fixture_only',
                })
                if entry['disposition'] in ('port', 'observable_guard', 'additional_guard'):
                    self.assertTrue(entry['cases'])
                else:
                    self.assertEqual(entry['cases'], [])

    def test_oracle_never_records_a_harness_failure_as_an_expected_browser_behavior(self):
        def check(value):
            if isinstance(value, dict):
                self.assertNotIn('harnessError', value)
                for child in value.values():
                    check(child)
            elif isinstance(value, list):
                for child in value:
                    check(child)
        check(json.loads(CONTRACTS.read_text()))

    def test_comparison_collects_all_failures_without_engine_specific_exceptions(self):
        results = []
        failures = compare_upstream(
            {'matched': True, 'different': {'found': False}, 'extra': 'unexpected'},
            {'matched': True, 'different': {'found': True}, 'missing': True},
            'disabled', results,
        )
        self.assertEqual(failures, ['disabled/different', 'disabled/extra', 'disabled/missing'])
        self.assertEqual([item['ok'] for item in results], [False, False, True, False])
        self.assertEqual(results[0]['chromium'], {'found': True})
        self.assertEqual(results[0]['observed'], {'found': False})

    def test_full_conformance_suite_remains_explicit_while_capabilities_are_missing(self):
        self.assertNotIn('web-security-upstream', DEFAULT_GROUP_NAMES)
        self.assertEqual([group.name for group in OPTIONAL_PROCESS_GROUPS], ['web-security-upstream'])
        selection = resolve_group_selection(['web-security-upstream'])
        self.assertEqual([group.name for group in selection.process_groups], ['web-security-upstream'])
        self.assertFalse(selection.needs_playwright)

    def test_file_origin_fixture_does_not_reflect_origin_over_a_deliberately_wrong_allow_header(self):
        fixture = FixtureServer()
        fixture.start()
        try:
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            for allow in ('null', 'file://'):
                with self.subTest(allow=allow):
                    request = urllib.request.Request(
                        fixture.url + '/web-security/echo?case=file-allow&allow=' + allow,
                        method='OPTIONS', headers={'Origin': 'null'},
                    )
                    with opener.open(request, timeout=3) as response:
                        self.assertEqual(response.headers['Access-Control-Allow-Origin'], allow)
                        self.assertEqual(response.headers['Access-Control-Allow-Headers'], 'X-NotSimple')
            self.assertEqual([item['origin'] for item in fixture.web_security.requests('file-allow')],
                             ['null', 'null'])
        finally:
            fixture.stop()

    def test_origin_agent_cluster_fixture_preserves_explicit_opt_in(self):
        fixture = FixtureServer()
        fixture.start()
        try:
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            with opener.open(fixture.url + '/web-security/page?oac=?1', timeout=3) as response:
                self.assertEqual(response.headers['Origin-Agent-Cluster'], '?1')
        finally:
            fixture.stop()
