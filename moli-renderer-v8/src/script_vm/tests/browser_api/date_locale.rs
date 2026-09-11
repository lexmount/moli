use super::*;

#[test]
fn date_and_intl_declared_wrappers_preserve_native_method_descriptors() {
    let mut vm = new_storage_test_vm("https://date-intl-declarations.test/");
    let probe = r#"JSON.stringify((() => {
      const constructors = ['Collator', 'DateTimeFormat', 'DisplayNames', 'DurationFormat',
        'ListFormat', 'NumberFormat', 'PluralRules', 'RelativeTimeFormat', 'Segmenter'];
      const methods = [[Date, 'parse', 1], ...constructors
        .filter(name => typeof Intl[name] === 'function')
        .map(name => [Intl[name].prototype, 'resolvedOptions', 0])];
      return methods.map(([owner, name, length]) => {
        const descriptor = Object.getOwnPropertyDescriptor(owner, name);
        const method = descriptor.value;
        let nonConstructible = false;
        // Test [[Construct]] without executing the method with an invalid receiver.
        try { Reflect.construct(function() {}, [], method); }
        catch (error) { nonConstructible = error instanceof TypeError; }
        return [method.name === name, method.length === length,
          !descriptor.enumerable, descriptor.writable, descriptor.configurable,
          !Object.hasOwn(method, 'prototype'), nonConstructible];
      });
    })())"#;
    for timezone in [None, Some("Europe/Paris"), None] {
        vm.set_timezone_override(timezone);
        let rows: Vec<Vec<bool>> = serde_json::from_str(&vm.eval(probe).unwrap()).unwrap();
        assert!(
            rows.len() >= 9,
            "Date and native Intl methods must be covered"
        );
        for (index, row) in rows.into_iter().enumerate() {
            assert_eq!(row, vec![true; 7], "method {index}, timezone {timezone:?}");
        }
    }
}

#[test]
fn emulation_constructor_envelopes_do_not_convert_page_arguments() {
    let mut vm = new_storage_test_vm("https://date-intl-raw-arguments.test/");
    let probe = r#"JSON.stringify((() => {
      const sentinel = {};
      const poison = {[Symbol.toPrimitive]() { throw sentinel; }};
      const ignored = typeof Date(poison) === 'string' &&
        typeof Date.call(poison, poison) === 'string';
      const conversions = [];
      const fields = [2024, 0, 2, 3, 4, 5, 6].map((value, index) => ({
        [Symbol.toPrimitive](hint) { conversions.push(`${index}:${hint}`); return value; }
      }));
      class DerivedDate extends Date {}
      const date = new DerivedDate(...fields, poison);
      const localFields = [date.getFullYear(), date.getMonth(), date.getDate(),
        date.getHours(), date.getMinutes(), date.getSeconds(), date.getMilliseconds()];
      class DerivedFormat extends Intl.DateTimeFormat {}
      const format = new DerivedFormat('en-US', {timeZone: 'UTC'});
      const callable = [Intl.Collator, Intl.DateTimeFormat, Intl.NumberFormat].every(
        Ctor => Ctor('en-US') instanceof Ctor);
      const constructOnly = ['DisplayNames', 'DurationFormat', 'ListFormat',
        'PluralRules', 'RelativeTimeFormat', 'Segmenter'].every(name => {
          const Ctor = Intl[name];
          if (typeof Ctor !== 'function') return true;
          try { Ctor(poison, poison); return false; }
          catch (error) { return error instanceof TypeError; }
        });
      return [ignored, conversions, localFields,
        Object.getPrototypeOf(date) === DerivedDate.prototype,
        Object.getPrototypeOf(format) === DerivedFormat.prototype,
        callable, constructOnly, Number.isNaN(Date.parse()),
        Number.isNaN(+new Date(undefined)), Number.isFinite(+new Date()),
        Date.parse('2024-01-01T00:00:00Z', poison) === 1704067200000];
    })())"#;
    for (locale, timezone) in [
        (None, None),
        (Some("fr_FR"), Some("America/New_York")),
        (Some("zh_Hant_TW"), Some("Asia/Shanghai")),
        (None, None),
    ] {
        vm.set_locale_override(locale);
        vm.set_timezone_override(timezone);
        assert_eq!(
            vm.eval(probe).unwrap(),
            r#"[true,["0:number","1:number","2:number","3:number","4:number","5:number","6:number"],[2024,0,2,3,4,5,6],true,true,true,true,true,true,true,true]"#,
            "locale {locale:?}, timezone {timezone:?}"
        );
    }
}

#[test]
fn emulation_private_declarations_do_not_invoke_inherited_setters() {
    let mut vm = new_storage_test_vm("https://date-intl-private-declarations.test/");
    vm.set_timezone_override(Some("Europe/Paris"));
    let result = vm
        .eval(
            r#"JSON.stringify((() => {
      const keys = ['get', 'timeZone', 'original', 'timezone'];
      const before = keys.map(key => Object.getOwnPropertyDescriptor(Object.prototype, key));
      const sentinel = {};
      // These setters must not participate in building the private handler,
      // callback data or default options objects.
      try {
        for (const key of keys) Object.defineProperty(Object.prototype, key, {
          set() { throw sentinel; }, configurable: true
        });
        const date = new Date('2024-01-01T00:00:00Z');
        const frozen = Object.freeze({timeZone: undefined});
        const explicit = {timeZone: 'Europe/Paris'};
        return [
          new Intl.DateTimeFormat('en-US').resolvedOptions().timeZone,
          new Intl.DateTimeFormat('en-US', frozen).resolvedOptions().timeZone,
          ...['toLocaleString', 'toLocaleDateString', 'toLocaleTimeString'].map(method =>
            date[method]('en-US') === date[method]('en-US', explicit) &&
            date[method]('en-US', frozen) === date[method]('en-US', explicit))
        ];
      } finally {
        keys.forEach((key, index) => {
          if (before[index]) Object.defineProperty(Object.prototype, key, before[index]);
          else delete Object.prototype[key];
        });
      }
    })())"#,
        )
        .unwrap();
    assert_eq!(result, r#"["Europe/Paris","Europe/Paris",true,true,true]"#);
}

#[test]
fn icu_locale_conversion_is_fallible_and_does_not_set_the_global_default() {
    let mut vm = new_storage_test_vm("https://icu-language-tag.test/");
    let baseline = vm
        .eval("new Intl.NumberFormat().resolvedOptions().locale")
        .unwrap();
    assert_eq!(
        v8::icu::language_tag_for_locale("en_US").as_deref(),
        Some("en-US")
    );
    assert_eq!(
        v8::icu::language_tag_for_locale("de_DE@collation=phonebook;numbers=latn").as_deref(),
        Some("de-DE-u-co-phonebk-nu-latn")
    );
    assert_eq!(
        v8::icu::language_tag_for_locale("en_US_POSIX").as_deref(),
        Some("en-US-u-va-posix")
    );
    assert_eq!(v8::icu::language_tag_for_locale("en_US\0fr_FR"), None);
    assert_eq!(
        vm.eval("new Intl.NumberFormat().resolvedOptions().locale")
            .unwrap(),
        baseline
    );
}

#[test]
fn emulation_icu_locale_ids_are_converted_before_default_intl_arguments() {
    let mut vm = new_storage_test_vm("https://icu-locale-defaults.test/");
    vm.set_timezone_override(Some("UTC"));
    for (locale, language_tag) in [
        ("en_US", "en-US"),
        ("fr_FR", "fr-FR"),
        ("zh_Hant_TW", "zh-Hant-TW"),
        ("en_US_POSIX", "en-US"),
        ("de_DE@collation=phonebook", "de-DE-u-co-phonebk"),
        ("th_TH@calendar=buddhist", "th-TH-u-ca-buddhist"),
        ("ar_EG@numbers=latn", "ar-EG-u-nu-latn"),
    ] {
        vm.set_locale_override(Some(locale));
        let result = vm.eval(&format!(
            r#"(() => {{
              const tag = {language_tag:?};
              const date = new Date('2024-01-01T00:00:00Z');
              return JSON.stringify([
                new Intl.NumberFormat().format(1234.5) === new Intl.NumberFormat(tag).format(1234.5),
                new Intl.DateTimeFormat().format(date) === new Intl.DateTimeFormat(tag).format(date),
                new Intl.Collator().compare('ä', 'ae') === new Intl.Collator(tag).compare('ä', 'ae'),
                date.toLocaleString() === date.toLocaleString(tag),
                date.toLocaleDateString() === date.toLocaleDateString(tag),
                date.toLocaleTimeString() === date.toLocaleTimeString(tag),
                new Intl.NumberFormat('ja-JP').resolvedOptions().locale === 'ja-JP'
              ]);
            }})()"#
        ));
        assert_eq!(
            result.unwrap(),
            "[true,true,true,true,true,true,true]",
            "{locale}"
        );
    }
}

#[test]
fn emulation_date_unicode_and_invalid_input_stays_invalid() {
    let mut vm = new_storage_test_vm("https://unicode-date-input.test/");
    let probe = r#"JSON.stringify([
      '你好', '🙂', '+你好啊', '-🙂abc', '2024-你好', '\ud800', '\udfff',
      '2024-13-01', '2024-01-32', '2024-01-01Tgarbage', '-000000-01-01T00:00:00'
    ].map(input => [Number.isNaN(Date.parse(input)), new Date(input).toString()]))"#;
    let expected = serde_json::to_string(&vec![(true, "Invalid Date"); 11]).unwrap();
    assert_eq!(vm.eval(probe).unwrap(), expected);
    for timezone in ["UTC", "Europe/Paris", "America/New_York", "Asia/Shanghai"] {
        vm.set_timezone_override(Some(timezone));
        assert_eq!(vm.eval(probe).unwrap(), expected, "{timezone}");
    }
}

#[test]
fn emulation_date_local_grammar_and_coercion_preserve_native_contracts() {
    let mut vm = new_storage_test_vm("https://local-date-input.test/");
    for timezone in ["UTC", "Europe/Paris", "America/New_York", "Asia/Shanghai"] {
        vm.set_timezone_override(Some(timezone));
        let result = vm
            .eval(
                r#"(() => {
          const inputs = [
            ['Tue Jan 02 2024 00:00:00', 2], ['Thu Jan 04 2024 00:00:00', 4],
            ['Sat Jan 06 2024 00:00:00', 6], ['Jan 02 2024 (PST)', 2],
            ['Jan 02 2024 (PST (ignored)', 2], [' 2024-01-02 ', 2],
            ['2024-01-02T00:00:00', 2], ['2024-01-02t00:00:00', 2],
            ['Jan 02 2024\0PST', 2], ['2024-01-02T00:00:00\0Z', 2]
          ];
          const localFields = inputs.every(([input, day]) =>
            Date.parse(input) === +new Date(2024, 0, day) &&
            +new Date(input) === +new Date(2024, 0, day));
          const conversions = [];
          const parsed = Date.parse({[Symbol.toPrimitive](hint) {
            conversions.push(hint); return 'Tue Jan 02 2024 00:00:00';
          }});
          const sentinel = {};
          let exceptionPreserved = false;
          try { Date.parse({toString() { throw sentinel; }}); }
          catch (error) { exceptionPreserved = error === sentinel; }
          let symbolRejected = false;
          try { Date.parse(Symbol()); }
          catch (error) { symbolRejected = error instanceof TypeError; }
          return JSON.stringify([localFields, conversions,
            parsed === +new Date(2024, 0, 2), exceptionPreserved, symbolRejected]);
        })()"#,
            )
            .unwrap();
        assert_eq!(result, r#"[true,["string"],true,true,true]"#, "{timezone}");
    }
}

#[test]
fn emulation_timezone_does_not_change_option_get_order_or_exceptions() {
    let mut vm = new_storage_test_vm("https://intl-option-read-order.test/");
    // Capture native behavior in this same V8, without hard-coding the list of
    // options that a particular V8 release implements or reads more than once.
    let probe = r#"JSON.stringify((() => {
      const date = new Date('2024-01-01T00:00:00Z');
      return ['Intl', 'toLocaleString', 'toLocaleDateString', 'toLocaleTimeString'].map(method => {
        const call = options => method === 'Intl'
          ? new Intl.DateTimeFormat('en-US', options) : date[method]('en-US', options);
        const reads = [];
        const receivers = [];
        const target = Object.freeze({timeZone: undefined,
          get year() { receivers.push(this === options); return 'numeric'; }
        });
        const options = new Proxy(target, {
          get(target, key, receiver) {
            reads.push(String(key)); return Reflect.get(target, key, receiver);
          }
        });
        call(options);
        const sentinel = {};
        let throwsSame = false;
        let count = 0;
        try { call(Object.freeze({get timeZone() { count++; throw sentinel; }})); }
        catch (error) { throwsSame = error === sentinel; }
        return [reads, receivers, throwsSame, count];
      });
    })())"#;
    let baseline = vm.eval(probe).unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_str(&baseline).unwrap();
    assert_eq!(rows.len(), 4);
    for row in rows {
        assert_eq!(row[2], true, "the original thrown value must propagate");
        assert_eq!(row[3], 1, "a throwing timeZone getter is read once");
    }
    vm.set_timezone_override(Some("Europe/Paris"));
    assert_eq!(vm.eval(probe).unwrap(), baseline);
    vm.set_timezone_override(None);
    assert_eq!(vm.eval(probe).unwrap(), baseline);
}

#[test]
fn emulation_timezone_preserves_frozen_options_and_original_getter_receivers() {
    let mut vm = new_storage_test_vm("https://frozen-intl-options.test/");
    vm.set_timezone_override(Some("Europe/Paris"));
    let result = vm.eval(r#"(() => {
      const date = new Date('2024-01-01T00:00:00Z');
      const frozen = Object.freeze({timeZone: undefined});
      const noGetter = Object.defineProperty({}, 'timeZone', {get: undefined});
      let receiver;
      const accessor = Object.freeze({get timeZone() { receiver = this; return undefined; }});
      const methods = ['toLocaleString', 'toLocaleDateString', 'toLocaleTimeString'];
      return JSON.stringify([
        new Intl.DateTimeFormat('en-US', frozen).resolvedOptions().timeZone,
        new Intl.DateTimeFormat('en-US', noGetter).resolvedOptions().timeZone,
        new Intl.DateTimeFormat('en-US', accessor).resolvedOptions().timeZone,
        receiver === accessor,
        ...methods.map(method => date[method]('en-US', frozen) === date[method]('en-US', {timeZone: 'Europe/Paris'})),
        new Intl.DateTimeFormat('en-US', Object.freeze({timeZone: 'UTC'})).resolvedOptions().timeZone,
        frozen.timeZone === undefined
      ]);
    })()"#).unwrap();
    assert_eq!(
        result,
        r#"["Europe/Paris","Europe/Paris","Europe/Paris",true,true,true,true,"UTC",true]"#
    );
}

#[test]
fn emulation_date_parsing_preserves_native_explicit_legacy_timezones() {
    let mut vm = new_storage_test_vm("https://legacy-date-zones.test/");
    vm.eval(
        r#"globalThis.explicitDateInputs = [
      'Mon Jan 01 2024 00:00:00 PST', 'Mon Jan 01 2024 00:00:00 EST',
      'Mon Jan 01 2024 00:00:00 PDT', 'Mon Jan 01 2024 00:00:00 EDT',
      'Mon Jan 01 2024 00:00:00 CST', 'Mon Jan 01 2024 00:00:00 CDT',
      'Mon Jan 01 2024 00:00:00 MST', 'Mon Jan 01 2024 00:00:00 MDT',
      'Mon Jan 01 2024 00:00:00 UT', 'Mon Jan 01 2024 00:00:00 UTC',
      'Mon Jan 01 2024 00:00:00 GMT', 'Mon Jan 01 2024 00:00:00 Z',
      'Mon Jan 01 2024 00:00:00 +0530', 'Mon Jan 01 2024 00:00:00 -04:00',
      '2024-01-01T00:00:00Z', '2024-01-01T00:00:00+02:00',
      '2024', '2024-01', '2024-01-01', '+002024-01-01', '2024-01-01\0PST',
      'Mon Jan 01 2024 00:00:00 PST (Pacific Standard Time)'
    ]; JSON.stringify(explicitDateInputs.map(Date.parse))"#,
    )
    .unwrap();
    let probe =
        "JSON.stringify(explicitDateInputs.map(input => [Date.parse(input), +new Date(input)]))";
    let baseline = vm.eval(probe).unwrap();
    for timezone in ["UTC", "Europe/Paris", "America/New_York", "Asia/Shanghai"] {
        vm.set_timezone_override(Some(timezone));
        assert_eq!(vm.eval(probe).unwrap(), baseline, "{timezone}");
    }
}
