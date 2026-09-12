use super::*;

#[test]
fn process_environment_defaults_reach_existing_and_new_isolates_without_wrapping_builtins() {
    let owner = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut peer = new_storage_test_vm("https://environment-peer.test/");
    let defaults = r#"JSON.stringify([
        new Intl.NumberFormat().resolvedOptions().locale,
        new Intl.DateTimeFormat().resolvedOptions().timeZone,
        new Date('2024-01-01T00:00:00Z').getTimezoneOffset()
    ])"#;
    let baseline = peer.eval(defaults).unwrap();
    peer.eval(
        r#"globalThis.originalDate = Date;
        globalThis.originalParse = Date.parse;
        globalThis.originalIntl = Intl.DateTimeFormat;
        globalThis.savedFormatter = new Intl.DateTimeFormat();
        globalThis.savedZone = savedFormatter.resolvedOptions().timeZone;
        globalThis.savedDate = new Date('2024-01-01T00:00:00Z');"#,
    )
    .unwrap();
    owner.set_locale(Some("fr_FR")).unwrap();
    owner.set_timezone(Some("Europe/Paris")).unwrap();
    assert_eq!(
        peer.eval(defaults).unwrap(),
        r#"["fr-FR","Europe/Paris",-60]"#
    );
    let mut late = new_storage_test_vm("https://environment-late.test/");
    assert_eq!(late.eval(defaults).unwrap(), peer.eval(defaults).unwrap());
    assert_eq!(
        peer.eval(
            r#"JSON.stringify([
        Date === originalDate, Date.parse === originalParse,
        Intl.DateTimeFormat === originalIntl,
        Function.prototype.toString.call(Date).includes('[native code]'),
        savedFormatter.resolvedOptions().timeZone === savedZone,
        savedDate.getTimezoneOffset() === -60
    ])"#
        )
        .unwrap(),
        "[true,true,true,true,true,true]"
    );
    drop(owner);
    assert_eq!(peer.eval(defaults).unwrap(), baseline);
    assert_eq!(late.eval(defaults).unwrap(), baseline);
}

#[test]
fn process_environment_claim_conflicts_and_failed_updates_leave_the_default_unchanged() {
    let peer_owner = moli_v8_platform::ProcessEnvironmentOwner::default();
    let owner = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut peer = new_storage_test_vm("https://environment-other.test/");
    let probe = r#"JSON.stringify([
        new Intl.NumberFormat().resolvedOptions().locale,
        new Intl.DateTimeFormat().resolvedOptions().timeZone
    ])"#;
    let baseline = peer.eval(probe).unwrap();
    owner.set_locale(Some("fr_FR")).unwrap();
    owner.set_timezone(Some("Europe/Paris")).unwrap();
    assert!(peer_owner.set_locale(Some("fr_FR")).is_err());
    assert!(peer_owner.set_locale(None).is_err());
    assert!(peer_owner.set_timezone(Some("Asia/Shanghai")).is_err());
    // Success without ownership must neither reset nor resurrect the claim.
    peer_owner.set_timezone(Some("Europe/Paris")).unwrap();
    peer_owner.set_timezone(None).unwrap();
    for invalid in ["_@invalid", "fr_FR\0en_US"] {
        assert!(owner.set_locale(Some(invalid)).is_err());
    }
    for invalid in ["Mars/Olympus", "Etc/Unknown", "UTC\0Europe/Paris"] {
        assert!(owner.set_timezone(Some(invalid)).is_err());
    }
    assert_eq!(peer.eval(probe).unwrap(), r#"["fr-FR","Europe/Paris"]"#);
    drop(owner);
    assert_eq!(peer.eval(probe).unwrap(), baseline);
    peer_owner.set_locale(Some("de_DE")).unwrap();
    peer_owner.set_timezone(Some("Asia/Shanghai")).unwrap();
    assert_eq!(peer.eval(probe).unwrap(), r#"["de-DE","Asia/Shanghai"]"#);
    peer_owner.set_locale(None).unwrap();
    peer_owner.set_timezone(None).unwrap();
    assert_eq!(peer.eval(probe).unwrap(), baseline);
}

#[test]
fn emulation_date_constructor_converts_objects_once_with_default_hint() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://date-object-conversion.test/");
    for timezone in ["UTC", "Europe/Paris", "America/New_York", "Asia/Shanghai"] {
        environment.set_timezone(Some(timezone)).unwrap();
        assert_eq!(vm.eval(r#"JSON.stringify((() => {
          const input = '2024-01-01T00:00:00';
          const expected = +new Date(2024, 0, 1);
          const calls = [];
          const objects = [new String(input), [input],
            {[Symbol.toPrimitive](hint) { calls.push(hint); return input; }},
            {valueOf() { calls.push('valueOf'); return {}; },
             toString() { calls.push('toString'); return input; }}];
          const dates = objects.map(value => +new Date(value) === expected);
          const original = new Date(1234);
          const sentinel = {};
          Object.defineProperty(original, Symbol.toPrimitive, {get() {throw sentinel;}});
          const copied = +new Date(original) === 1234;
          const primitiveValues = [1234, null, true, false, undefined];
          const numeric = primitiveValues.every(value =>
            Object.is(+new Date({[Symbol.toPrimitive]() {return value;}}), +new Date(value)));
          const errors = [
            [{[Symbol.toPrimitive]() {throw sentinel;}}, sentinel],
            [{get [Symbol.toPrimitive]() {throw sentinel;}}, sentinel],
            [{[Symbol.toPrimitive]: 1}, TypeError],
            [{[Symbol.toPrimitive]() {return {};}}, TypeError],
            [{valueOf() {return {};}, toString() {return {};}}, TypeError],
            [{[Symbol.toPrimitive]() {return Symbol();}}, TypeError],
            [{[Symbol.toPrimitive]() {return 1n;}}, TypeError]
          ].every(([value, expected]) => {
            try { new Date(value); return false; }
            catch (error) {return expected === sentinel ? error === sentinel : error instanceof expected;}
          });
          return [dates, calls, copied, numeric, errors];
        })())"#).unwrap(),
        r#"[[true,true,true,true],["default","valueOf","toString"],true,true,true]"#, "{timezone}");
    }
}

#[test]
fn emulation_locale_fallback_covers_empty_and_unmatched_lists() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://intl-locale-fallback.test/");
    environment.set_locale(Some("fr_FR")).unwrap();
    environment.set_timezone(Some("Europe/Paris")).unwrap();
    assert_eq!(vm.eval(r#"JSON.stringify((() => {
      const constructors = ['Collator', 'DateTimeFormat', 'DisplayNames', 'DurationFormat',
        'ListFormat', 'NumberFormat', 'PluralRules', 'RelativeTimeFormat', 'Segmenter'];
      const date = new Date('2024-01-01T00:00:00Z');
      const cases = [undefined, [], {}, ['zz-ZZ']];
      const resolved = constructors.every(name => {
        const Ctor = Intl[name];
        if (!Ctor) return true;
        return ['lookup', 'best fit'].every(localeMatcher => {
          const options = {localeMatcher, type: name === 'DisplayNames' ? 'language' : undefined};
          const expected = new Ctor(undefined, options).resolvedOptions().locale;
          return cases.every(locales => new Ctor(locales, options).resolvedOptions().locale === expected);
        });
      });
      const numbers = cases.every(locales => new Intl.NumberFormat(locales).format(1234.5) ===
        new Intl.NumberFormat('fr-FR').format(1234.5));
      const dates = ['toLocaleString', 'toLocaleDateString', 'toLocaleTimeString'].every(method =>
        cases.every(locales => date[method](locales) === date[method]('fr-FR')));
      const explicit = [new Intl.Locale('en-US'), ['zz-ZZ', 'en-US'], ['en-US', 'fr-FR']].every(
        locales => new Intl.NumberFormat(locales).format(1234.5) === '1,234.5');
      return [resolved, numbers, dates, explicit];
    })())"#).unwrap(), "[true,true,true,true]");
}

#[test]
fn emulation_locale_fallback_preserves_native_observation_order() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://intl-locale-coercion.test/");
    let probe = r#"JSON.stringify((() => {
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
    })())"#;
    let baseline = vm.eval(probe).unwrap();
    environment.set_locale(Some("fr_FR")).unwrap();
    environment.set_timezone(Some("Europe/Paris")).unwrap();
    assert_eq!(vm.eval(probe).unwrap(), baseline);
    assert_eq!(
        vm.eval(
            r#"JSON.stringify((() => {
      const sentinel = {};
      const poison = new Proxy({}, {get() {throw sentinel;}});
      const invalidDate = new Date(NaN).toLocaleString(poison, poison) === 'Invalid Date';
      let getterError = false;
      try {new Intl.NumberFormat(poison);} catch (error) {getterError = error === sentinel;}
      const invalidLocales = [null, ['en_US'], [1]].every(locales => {
        try {new Intl.NumberFormat(locales); return false;}
        catch (error) {return error instanceof TypeError || error instanceof RangeError;}
      });
      return [invalidDate, getterError, invalidLocales];
    })())"#
        )
        .unwrap(),
        "[true,true,true]"
    );
}

#[test]
fn process_environment_defaults_do_not_read_array_prototype() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://intl-private-arguments.test/");
    environment.set_locale(Some("fr_FR")).unwrap();
    environment.set_timezone(Some("Europe/Paris")).unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
      const sentinel = {};
      const zero = Object.getOwnPropertyDescriptor(Array.prototype, '0');
      const one = Object.getOwnPropertyDescriptor(Array.prototype, '1');
      try {
        for (const key of ['0', '1']) Object.defineProperty(Array.prototype, key, {
          get() {throw sentinel;}, set() {throw sentinel;}, configurable: true
        });
        return new Intl.NumberFormat().resolvedOptions().locale === 'fr-FR' &&
          new Intl.DateTimeFormat().resolvedOptions().timeZone === 'Europe/Paris' &&
          new Intl.DateTimeFormat(undefined, {}).resolvedOptions().timeZone === 'Europe/Paris';
      } finally {
        if (zero) Object.defineProperty(Array.prototype, '0', zero); else delete Array.prototype[0];
        if (one) Object.defineProperty(Array.prototype, '1', one); else delete Array.prototype[1];
      }
    })()"#
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn process_environment_preserves_native_primitive_options_and_defaults() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://intl-primitive-options.test/");
    environment.set_timezone(Some("Europe/Paris")).unwrap();
    assert_eq!(vm.eval(r#"JSON.stringify((() => {
      const date = new Date('2024-01-01T00:00:00Z');
      const options = [42, true, '', Symbol(), 1n];
      const zones = options.every(value =>
        new Intl.DateTimeFormat('en-US', value).resolvedOptions().timeZone === 'Europe/Paris');
      const dates = ['toLocaleString', 'toLocaleDateString', 'toLocaleTimeString'].every(method =>
        options.every(value => date[method]('en-US', value) === date[method]('en-US', {timeZone: 'Europe/Paris'})));
      let nullRejected = false;
      try {new Intl.DateTimeFormat('en-US', null);} catch (error) {nullRejected = error instanceof TypeError;}
      return [zones, dates, nullRejected];
    })())"#).unwrap(), "[true,true,true]");
}

#[test]
fn emulation_date_time_first_legacy_hyphens_are_not_timezone_offsets() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://date-time-first.test/");
    for timezone in ["Europe/Paris", "America/New_York", "Asia/Shanghai"] {
        environment.set_timezone(Some(timezone)).unwrap();
        assert_eq!(
            vm.eval(
                r#"JSON.stringify([
          '00:00:00 Jan-01-2024', '00:00:00 01-01-2024',
          '00:00 Jan-01-2024', '00:00:00.000 Jan-01-2024',
          '00:00:00 January-01-2024', '00:00:00 Jan-01-2024 (PST)'
        ].map(input => [Date.parse(input) === +new Date(2024, 0, 1),
          +new Date(input) === +new Date(2024, 0, 1)]))"#
            )
            .unwrap(),
            "[[true,true],[true,true],[true,true],[true,true],[true,true],[true,true]]",
            "{timezone}"
        );
    }
}

#[test]
fn date_and_intl_native_method_descriptors_are_unchanged() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
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
        environment.set_timezone(timezone).unwrap();
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
fn process_environment_preserves_native_date_and_intl_coercion() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
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
        environment.set_locale(locale).unwrap();
        environment.set_timezone(timezone).unwrap();
        assert_eq!(
            vm.eval(probe).unwrap(),
            r#"[true,["0:number","1:number","2:number","3:number","4:number","5:number","6:number"],[2024,0,2,3,4,5,6],true,true,true,true,true,true,true,true]"#,
            "locale {locale:?}, timezone {timezone:?}"
        );
    }
}

#[test]
fn process_environment_preserves_native_options_with_inherited_setters() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://date-intl-inherited-options.test/");
    environment.set_timezone(Some("Europe/Paris")).unwrap();
    let result = vm
        .eval(
            r#"JSON.stringify((() => {
      const keys = ['get', 'timeZone'];
      const before = keys.map(key => Object.getOwnPropertyDescriptor(Object.prototype, key));
      const sentinel = {};
      // Changing native defaults must not add observable property writes to
      // option handling, including frozen options and inherited accessors.
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
fn native_icu_locale_ids_drive_default_intl_behavior() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://icu-locale-defaults.test/");
    environment.set_timezone(Some("UTC")).unwrap();
    for (locale, language_tag) in [
        ("en_US", "en-US"),
        ("fr_FR", "fr-FR"),
        ("zh_Hant_TW", "zh-Hant-TW"),
        ("en_US_POSIX", "en-US"),
        ("de_DE@collation=phonebook", "de-DE-u-co-phonebk"),
        ("th_TH@calendar=buddhist", "th-TH-u-ca-buddhist"),
        ("ar_EG@numbers=latn", "ar-EG-u-nu-latn"),
    ] {
        environment.set_locale(Some(locale)).unwrap();
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
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://unicode-date-input.test/");
    let probe = r#"JSON.stringify([
      '你好', '🙂', '+你好啊', '-🙂abc', '2024-你好', '\ud800', '\udfff',
      '2024-13-01', '2024-01-32', '2024-01-01Tgarbage', '-000000-01-01T00:00:00'
    ].map(input => [Number.isNaN(Date.parse(input)), new Date(input).toString()]))"#;
    let expected = serde_json::to_string(&vec![(true, "Invalid Date"); 11]).unwrap();
    assert_eq!(vm.eval(probe).unwrap(), expected);
    for timezone in ["UTC", "Europe/Paris", "America/New_York", "Asia/Shanghai"] {
        environment.set_timezone(Some(timezone)).unwrap();
        assert_eq!(vm.eval(probe).unwrap(), expected, "{timezone}");
    }
}

#[test]
fn emulation_date_local_grammar_and_coercion_preserve_native_contracts() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://local-date-input.test/");
    for timezone in ["UTC", "Europe/Paris", "America/New_York", "Asia/Shanghai"] {
        environment.set_timezone(Some(timezone)).unwrap();
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
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
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
    environment.set_timezone(Some("Europe/Paris")).unwrap();
    assert_eq!(vm.eval(probe).unwrap(), baseline);
    environment.set_timezone(None).unwrap();
    assert_eq!(vm.eval(probe).unwrap(), baseline);
}

#[test]
fn emulation_timezone_preserves_frozen_options_and_original_getter_receivers() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://frozen-intl-options.test/");
    environment.set_timezone(Some("Europe/Paris")).unwrap();
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
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
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
        environment.set_timezone(Some(timezone)).unwrap();
        assert_eq!(vm.eval(probe).unwrap(), baseline, "{timezone}");
    }
}
