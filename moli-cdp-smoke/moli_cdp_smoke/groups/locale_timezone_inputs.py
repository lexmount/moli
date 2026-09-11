from __future__ import annotations

import contextlib
from typing import Any

from ..assertions import assert_equal, record


EXPLICIT_DATE_INPUTS = [
    *(f"Mon Jan 01 2024 00:00:00 {zone}" for zone in
      ["PST", "PDT", "EST", "EDT", "CST", "CDT", "MST", "MDT", "UT", "UTC", "GMT", "Z"]),
    "Mon Jan 01 2024 00:00:00 +0530",
    "Mon Jan 01 2024 00:00:00 -04:00",
    "Mon Jan 01 2024 00:00:00 PST (Pacific Standard Time)",
    "Mon Jan 01 2024 00:00:00 pSt\u00a0",
    "00:00:00-0100 Jan-01-2024",
    "00:00-01:00 01-01-2024",
    "00:00:00.000-0100 Jan-01-2024",
    "2024-01-01T00:00:00Z",
    "2024-01-01T00:00:00+02:00",
    "2024", "2024-01", "2024-01-01", "+002024-01-01",
    "2024-01-01\0PST",
]

DATE_VALUES = "inputs => inputs.map(input => [Date.parse(input), +new Date(input)])"

OPTION_READS = """() => {
  const date = new Date('2024-01-01T00:00:00Z');
  return ['Intl', 'toLocaleString', 'toLocaleDateString', 'toLocaleTimeString'].map(method => {
    const reads = [];
    const receivers = [];
    const target = Object.freeze({
      timeZone: undefined,
      get year() { receivers.push(this === options); return 'numeric'; },
      get month() { receivers.push(this === options); return '2-digit'; }
    });
    const options = new Proxy(target, {
      get(target, key, receiver) {
        reads.push(String(key));
        return Reflect.get(target, key, receiver);
      }
    });
    if (method === 'Intl') new Intl.DateTimeFormat('en-US', options);
    else date[method]('en-US', options);
    return [method, reads, receivers];
  });
}"""

OBJECT_DATES = """() => {
  const input = '2024-01-01T00:00:00';
  const calls = [];
  const values = [new String(input), [input],
    {[Symbol.toPrimitive](hint) {calls.push(hint); return input;}},
    {valueOf() {calls.push('valueOf'); return {};},
     toString() {calls.push('toString'); return input;}}];
  const correct = values.map(value => +new Date(value) === +new Date(2024, 0, 1));
  const original = new Date(1234);
  Object.defineProperty(original, Symbol.toPrimitive, {get() {throw new Error('unexpected Date coercion');}});
  return [correct, calls, +new Date(original),
    +new Date({[Symbol.toPrimitive]() {return 1234;}})];
}"""

PRIMITIVE_OPTIONS = """zone => {
  const date = new Date('2024-01-01T00:00:00Z');
  return [42, true, '', Symbol(), 1n].map(options => [
    new Intl.DateTimeFormat('en-US', options).resolvedOptions().timeZone === zone,
    ...['toLocaleString', 'toLocaleDateString', 'toLocaleTimeString'].map(method =>
      date[method]('en-US', options) === date[method]('en-US', {timeZone: zone}))
  ]);
}"""

LOCALE_FALLBACK = """() => {
  const date = new Date('2024-01-01T00:00:00Z');
  const names = ['Collator', 'DateTimeFormat', 'DisplayNames', 'DurationFormat',
    'ListFormat', 'NumberFormat', 'PluralRules', 'RelativeTimeFormat', 'Segmenter'];
  return [[], {}, ['zz-ZZ']].map(locales => [
    ...names.filter(name => typeof Intl[name] === 'function').map(name =>
      ['lookup', 'best fit'].every(localeMatcher => {
        const options = {localeMatcher, type: name === 'DisplayNames' ? 'language' : undefined};
        return new Intl[name](locales, options).resolvedOptions().locale ===
          new Intl[name](undefined, options).resolvedOptions().locale;
      })),
    new Intl.NumberFormat(locales).format(1234.5) === new Intl.NumberFormat().format(1234.5),
    ...['toLocaleString', 'toLocaleDateString', 'toLocaleTimeString'].map(method =>
      date[method](locales) === date[method]())
  ]);
}"""

LOCALE_READS = """() => {
  const date = new Date(0);
  return ['Collator', 'DateTimeFormat', 'NumberFormat', 'PluralRules',
    'toLocaleString', 'toLocaleDateString', 'toLocaleTimeString'].map(name => {
    const trace = [];
    const locales = new Proxy({length: 1, 0: {toString() {trace.push('locale:string'); return 'zz-ZZ';}}}, {
      get(target, key, receiver) {trace.push(`locales:get:${String(key)}`); return Reflect.get(target, key, receiver);},
      has(target, key) {trace.push(`locales:has:${String(key)}`); return Reflect.has(target, key);}
    });
    const options = new Proxy({localeMatcher: {toString() {trace.push('matcher:string'); return 'lookup';}}}, {
      get(target, key, receiver) {trace.push(`options:get:${String(key)}`); return Reflect.get(target, key, receiver);}
    });
    if (name.startsWith('toLocale')) date[name](locales, options);
    else {
      const newTarget = new Proxy(function() {}, {get(target, key, receiver) {
        trace.push(`newTarget:get:${String(key)}`); return Reflect.get(target, key, receiver);
      }});
      Reflect.construct(Intl[name], [locales, options], newTarget);
    }
    return trace;
  });
}"""


async def run_locale_timezone_inputs_group(
    browser: Any, _fixture: str, results: list[dict[str, Any]]
) -> None:
    """Input-boundary contracts, runnable unchanged against Moli or Chromium.

    Keep this group process-isolated: the Unicode regression used to panic in
    a native callback. Each completed matrix row is recorded before advancing.
    Native explicit-zone parsing and option-access traces are captured before
    emulation so host timezone and V8 option-list differences are not goldens.
    """
    context = await browser.new_context()
    cdp = None
    try:
        page = await context.new_page()
        await page.goto("data:text/html,<!doctype html><title>Date Intl inputs</title>")
        cdp = await context.new_cdp_session(page)
        baseline_dates = await page.evaluate(DATE_VALUES, EXPLICIT_DATE_INPUTS)
        assert_equal(
            all(value is not None for pair in baseline_dates for value in pair),
            True, "explicit-zone reference inputs must be valid native dates",
        )
        baseline_reads = await page.evaluate(OPTION_READS)
        baseline_locale_reads = await page.evaluate(LOCALE_READS)
        baseline_defaults = await page.evaluate("""() => [
          new Intl.NumberFormat().resolvedOptions().locale,
          new Intl.DateTimeFormat().resolvedOptions().timeZone,
          new Date(0).getTimezoneOffset()
        ]""")
        for zone in ["UTC", "Europe/Paris", "America/New_York", "Asia/Shanghai"]:
            await cdp.send("Emulation.setTimezoneOverride", {"timezoneId": zone})
            unicode_result = await page.evaluate(r"""() => [
              '你好', '🙂', '+你好啊', '-🙂abc', '2024-你好', '\ud800', '\udfff'
            ].map(input => [Number.isNaN(Date.parse(input)), new Date(input).toString()])""")
            assert_equal(unicode_result, [[True, "Invalid Date"]] * 7, f"Unicode dates under {zone}")
            record(results, "timezone_unicode_dates", {"timezone": zone, "inputs": 7})

            dates = await page.evaluate(DATE_VALUES, EXPLICIT_DATE_INPUTS)
            assert_equal(dates, baseline_dates, f"explicit date zones survive {zone}")
            record(results, "timezone_explicit_dates", {"timezone": zone, "inputs": len(dates)})

            assert_equal(await page.evaluate(OBJECT_DATES),
                         [[True] * 4, ['default', 'valueOf', 'toString'], 1234, 1234],
                         f"single Date argument coercion under {zone}")
            record(results, "timezone_date_object_coercion", {"timezone": zone})
            assert_equal(await page.evaluate(PRIMITIVE_OPTIONS, zone), [[True] * 4] * 5,
                         f"primitive Intl/Date options under {zone}")
            record(results, "timezone_primitive_options", {"timezone": zone, "inputs": 5})

            local_dates = await page.evaluate(r"""() => [
              ['Tue Jan 02 2024 00:00:00', 2],
              ['Thu Jan 04 2024 00:00:00', 4],
              ['Sat Jan 06 2024 00:00:00', 6],
              ['Jan 02 2024 (PST)', 2],
              ['Jan 02 2024 (PST (ignored)', 2],
              [' 2024-01-02 ', 2],
              ['2024-01-02T00:00:00', 2],
              ['2024-01-02t00:00:00', 2],
              ['00:00:00 Jan-02-2024', 2],
              ['00:00:00 01-02-2024', 2],
              ['00:00 Jan-02-2024', 2],
              ['00:00:00.000 January-02-2024 (PST)', 2],
              ['Jan 02 2024\0PST', 2],
              ['2024-01-02T00:00:00\0Z', 2]
            ].map(([input, day]) => [
              Date.parse(input) === +new Date(2024, 0, day),
              +new Date(input) === +new Date(2024, 0, day)
            ])""")
            assert_equal(local_dates, [[True, True]] * 14, f"local ISO/legacy fields under {zone}")
            record(results, "timezone_local_date_grammars", {"timezone": zone, "inputs": 14})

            frozen = await page.evaluate("""zone => {
              const date = new Date('2024-01-01T00:00:00Z');
              const options = Object.freeze({timeZone: undefined});
              const noGetter = Object.defineProperty({}, 'timeZone', {get: undefined});
              return [
                new Intl.DateTimeFormat('en-US', options).resolvedOptions().timeZone === zone,
                new Intl.DateTimeFormat('en-US', noGetter).resolvedOptions().timeZone === zone,
                new Intl.DateTimeFormat('en-US', Object.freeze({timeZone: 'UTC'})).resolvedOptions().timeZone === 'UTC',
                ...['toLocaleString', 'toLocaleDateString', 'toLocaleTimeString'].map(method =>
                  date[method]('en-US', options) === date[method]('en-US', {timeZone: zone})),
                options.timeZone === undefined && !Object.isExtensible(options)
              ];
            }""", zone)
            assert_equal(frozen, [True] * 7, f"frozen options under {zone}")
            assert_equal(await page.evaluate(OPTION_READS), baseline_reads, f"native option read order/receiver under {zone}")
            record(results, "timezone_frozen_options_and_getters", {"timezone": zone})

        for locale, tag in [
            ("en_US", "en-US"), ("fr_FR", "fr-FR"),
            ("zh_Hant_TW", "zh-Hant-TW"),
            # V8's DefaultLocale deliberately maps ICU's POSIX locale to en-US.
            ("en_US_POSIX", "en-US"),
            ("de_DE@collation=phonebook", "de-DE-u-co-phonebk"),
            ("th_TH@calendar=buddhist", "th-TH-u-ca-buddhist"),
            ("ar_EG@numbers=latn", "ar-EG-u-nu-latn"),
        ]:
            await cdp.send("Emulation.setLocaleOverride", {"locale": locale})
            result = await page.evaluate("""tag => {
              const date = new Date('2024-01-01T00:00:00Z');
              return [
                new Intl.NumberFormat().format(1234.5) === new Intl.NumberFormat(tag).format(1234.5),
                new Intl.DateTimeFormat().format(date) === new Intl.DateTimeFormat(tag).format(date),
                new Intl.Collator().compare('ä', 'ae') === new Intl.Collator(tag).compare('ä', 'ae'),
                ...['toLocaleString', 'toLocaleDateString', 'toLocaleTimeString'].map(method =>
                  date[method]() === date[method](tag)),
                new Intl.NumberFormat('ja-JP').resolvedOptions().locale === 'ja-JP',
                Intl.getCanonicalLocales(new Intl.NumberFormat().resolvedOptions().locale).length === 1,
                (() => { try { new Intl.NumberFormat('en_US'); return false; }
                  catch (error) { return error instanceof RangeError; } })()
              ];
            }""", tag)
            assert_equal(result, [True] * 9, f"ICU locale {locale} matches explicit {tag}")
            record(results, "icu_locale_default_conversion", {"locale": locale, "language_tag": tag})

        # A supported plain default makes fallback identity independent of
        # service-specific support for Unicode locale extensions above.
        await cdp.send("Emulation.setLocaleOverride", {"locale": "fr_FR"})
        fallback = await page.evaluate(LOCALE_FALLBACK)
        assert_equal(len(fallback), 3, "all empty/unsupported locale list variants exercised")
        for index, row in enumerate(fallback):
            assert_equal(len(row) >= 12, True, "Intl services and Date locale methods exercised")
            assert_equal(row, [True] * len(row), f"default locale fallback variant {index}")
        record(results, "locale_empty_and_unsupported_lists", {"locale": "fr_FR", "inputs": 3})
        assert_equal(await page.evaluate(LOCALE_READS), baseline_locale_reads,
                     "locale fallback preserves native newTarget, locale and option observation order")
        record(results, "locale_fallback_getter_order", {"locale": "fr_FR", "surfaces": 7})

        await cdp.send("Emulation.setLocaleOverride", {"locale": ""})
        await cdp.send("Emulation.setTimezoneOverride", {"timezoneId": ""})
        restored = await page.evaluate("""() => [
          new Intl.NumberFormat().resolvedOptions().locale,
          new Intl.DateTimeFormat().resolvedOptions().timeZone,
          new Date(0).getTimezoneOffset()
        ]""")
        assert_equal(restored, baseline_defaults, "clearing input-matrix overrides restores native defaults")
        record(results, "locale_timezone_input_matrix_reset")
    finally:
        if cdp is not None:
            with contextlib.suppress(Exception):
                await cdp.detach()
        await context.close()
