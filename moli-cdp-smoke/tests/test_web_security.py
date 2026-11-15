from __future__ import annotations

import unittest
import urllib.request

from moli_cdp_smoke.fixture import FixtureServer


class WebSecurityFixtureTests(unittest.TestCase):
    def test_live_fixture_records_preflight_body_origin_and_cookie_without_cors_opt_in(self):
        fixture = FixtureServer()
        fixture.start()
        try:
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            url = fixture.url + '/web-security/echo?case=witness'
            for method, body in (('OPTIONS', None), ('PUT', b'payload')):
                request = urllib.request.Request(url, data=body, method=method, headers={
                    'Origin':'http://origin.test', 'Cookie':'target=present', 'X-Author':'kept',
                })
                with opener.open(request,timeout=3) as response:
                    self.assertEqual(response.read(),b'payload')
                    self.assertEqual(response.headers['X-Secret'],'visible')
                    self.assertIsNone(response.headers['Access-Control-Allow-Origin'])
            self.assertEqual(fixture.web_security.requests('witness'),[
                {'path':'/web-security/echo','method':method,'origin':'http://origin.test',
                 'cookie':{'target':'present'},'author':'kept','body':body}
                for method,body in (('OPTIONS',''),('PUT','payload'))
            ])
        finally:
            fixture.stop()

    def test_cache_witness_has_one_explicit_cache_control_and_redirect_location(self):
        fixture = FixtureServer()
        fixture.start()
        try:
            class NoRedirect(urllib.request.HTTPRedirectHandler):
                def redirect_request(self, *args, **kwargs):
                    return None

            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),NoRedirect())
            with self.assertRaises(urllib.error.HTTPError) as raised:
                opener.open(fixture.url+'/web-security/redirect?case=cache&cache=1&to=/next',timeout=3)
            response = raised.exception
            try:
                self.assertEqual(response.code,302)
                self.assertEqual(response.headers.get_all('Cache-Control'),['public, max-age=3600'])
                self.assertEqual(response.headers['Location'],'/next')
                self.assertEqual(fixture.web_security.requests('cache')[0]['method'],'GET')
            finally:
                response.close()
        finally:
            fixture.stop()
