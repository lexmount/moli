use super::*;

#[test]
fn date_locale_methods_use_shared_time_formatting_surface() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://date-locale-formatting.test/");
    environment.set_timezone(Some("UTC")).unwrap();

    let us = vm
        .eval("new Date(0).toLocaleString()")
        .expect("default Date locale formatting should evaluate");
    assert_eq!(us, "1/1/1970, 12:00:00 AM");

    environment.set_locale(Some("fr-FR")).unwrap();
    environment.set_timezone(Some("Asia/Shanghai")).unwrap();

    let result = vm
        .eval(
            r#"
(() => {
  const date = new Date(0);
  const internalNames = () => Object.getOwnPropertyNames(globalThis)
    .filter(name => name === "__moliLocaleOverride" || name === "__moliTimeZoneOverride")
    .sort()
    .join(",");
  const initialSlots = internalNames();
  globalThis.__moliLocaleOverride = "en-US";
  globalThis.__moliTimeZoneOverride = "UTC";
  return JSON.stringify({
    initialSlots,
    spoofedSlots: internalNames(),
    intl: [
      Intl.DateTimeFormat().resolvedOptions().locale,
      Intl.DateTimeFormat().resolvedOptions().timeZone
    ].join("|"),
    formatted: [
      date.toLocaleString(),
      date.toLocaleDateString(),
      date.toLocaleTimeString(),
      new Date(NaN).toLocaleString()
    ].join("|")
  });
})()
"#,
        )
        .expect("Date locale methods should evaluate");

    assert_eq!(
        result,
        r#"{"initialSlots":"","spoofedSlots":"__moliLocaleOverride,__moliTimeZoneOverride","intl":"fr-FR|Asia/Shanghai","formatted":"01/01/1970 08:00:00|01/01/1970|08:00:00|Invalid Date"}"#
    );
}
#[test]
fn emulation_defaults_drive_real_intl_and_local_date_operations() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://date-locale-emulation-surface.test/");
    let baseline = vm
        .eval(
            r#"
(() => {
  const date = new Date('2024-01-01T00:00:00Z');
  return JSON.stringify([
    new Intl.NumberFormat().resolvedOptions().locale,
    new Intl.DateTimeFormat().resolvedOptions().timeZone,
    date.getTimezoneOffset(),
    date.getFullYear(),
    date.getMonth(),
    date.getDate(),
    date.getDay(),
    date.getHours(),
    date.getMinutes(),
    date.getSeconds(),
    date.getMilliseconds(),
    date.toString(),
    date.toDateString(),
    date.toTimeString()
  ]);
})()
"#,
        )
        .expect("baseline Intl and Date surfaces should evaluate");
    environment.set_locale(Some("fr-FR")).unwrap();
    environment.set_timezone(Some("Europe/Paris")).unwrap();

    let result = vm
        .eval(
            r#"
(() => {
  const winter = new Date('2024-01-01T00:00:00Z');
  const summer = new Date('2024-07-01T00:00:00Z');
  return JSON.stringify({
    locales: [
      new Intl.Collator().resolvedOptions().locale,
      new Intl.DateTimeFormat().resolvedOptions().locale,
      new Intl.NumberFormat().resolvedOptions().locale,
      new Intl.PluralRules().resolvedOptions().locale
    ],
    timezone: new Intl.DateTimeFormat().resolvedOptions().timeZone,
    formattedAsFrench: new Intl.NumberFormat().format(1.5).endsWith(',5'),
    winter: [winter.getTimezoneOffset(), winter.getHours(), winter.getDate()],
    summer: [summer.getTimezoneOffset(), summer.getHours(), summer.getDate()],
    strings: [winter.toString(), winter.toDateString(), winter.toTimeString()],
    explicit: [
      new Intl.NumberFormat('en-US').resolvedOptions().locale,
      new Intl.DateTimeFormat('en-US', { timeZone: 'UTC' }).resolvedOptions().timeZone
    ],
    invalidOptions: (() => {
      try { new Intl.DateTimeFormat(undefined, null); return 'accepted'; }
      catch (error) { return error.name; }
    })(),
    navigatorLanguage: navigator.language
  });
})()
"#,
        )
        .expect("Intl and Date emulation surfaces should evaluate");

    assert_eq!(
        result,
        r#"{"locales":["fr-FR","fr-FR","fr-FR","fr-FR"],"timezone":"Europe/Paris","formattedAsFrench":true,"winter":[-60,1,1],"summer":[-120,2,1],"strings":["Mon Jan 01 2024 01:00:00 GMT+0100 (heure normale d’Europe centrale)","Mon Jan 01 2024","01:00:00 GMT+0100 (heure normale d’Europe centrale)"],"explicit":["en-US","UTC"],"invalidOptions":"TypeError","navigatorLanguage":"en-US"}"#
    );

    environment.set_locale(None).unwrap();
    environment.set_timezone(None).unwrap();
    let restored = vm
        .eval(
            r#"
(() => {
  const date = new Date('2024-01-01T00:00:00Z');
  return JSON.stringify([
    new Intl.NumberFormat().resolvedOptions().locale,
    new Intl.DateTimeFormat().resolvedOptions().timeZone,
    date.getTimezoneOffset(),
    date.getFullYear(),
    date.getMonth(),
    date.getDate(),
    date.getDay(),
    date.getHours(),
    date.getMinutes(),
    date.getSeconds(),
    date.getMilliseconds(),
    date.toString(),
    date.toDateString(),
    date.toTimeString()
  ]);
})()
"#,
        )
        .expect("restored Intl and Date surfaces should evaluate");
    assert_eq!(restored, baseline);
}
#[test]
fn emulation_preserves_intl_construction_and_complete_local_date_operations() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://date-locale-native-semantics.test/");
    environment.set_locale(Some("fr-FR")).unwrap();
    environment.set_timezone(Some("America/New_York")).unwrap();

    let result = vm
        .eval(
            r#"
(() => {
  let timeZoneReads = 0;
  let getterReceiverPreserved = false;
  const options = {
    get timeZone() {
      timeZoneReads += 1;
      getterReceiverPreserved = this === options;
      return undefined;
    }
  };
  const optionFormat = new Intl.DateTimeFormat(undefined, options);

  class DerivedNumberFormat extends Intl.NumberFormat {}
  const numberFormat = new DerivedNumberFormat();
  class DerivedDate extends Date {}
  const derivedDate = new DerivedDate(2024, 0, 1);

  const setter = new Date('2024-01-01T05:00:00Z');
  const setterResult = setter.setHours(12, 34, 56, 789);
  const gapSetter = new Date('2024-03-10T06:30:00Z');
  gapSetter.setHours(2, 30, 0, 0);
  const revived = new Date(NaN);
  revived.setFullYear(2024, 0, 1);

  const explicitLocaleOptions = {
    timeZone: 'UTC', year: 'numeric', month: '2-digit', day: '2-digit',
    hour: '2-digit', minute: '2-digit', second: '2-digit', hourCycle: 'h23'
  };
  const explicitLocaleDate = new Date('2024-01-01T00:00:00Z');

  return JSON.stringify({
    intlSubclass: [
      numberFormat instanceof DerivedNumberFormat,
      numberFormat instanceof Intl.NumberFormat,
      Object.getPrototypeOf(numberFormat) === DerivedNumberFormat.prototype,
      numberFormat.resolvedOptions().locale
    ],
    options: [
      timeZoneReads,
      getterReceiverPreserved,
      optionFormat.resolvedOptions().timeZone
    ],
    dateSubclass: [
      derivedDate instanceof DerivedDate,
      derivedDate instanceof Date,
      Object.getPrototypeOf(derivedDate) === DerivedDate.prototype,
      derivedDate.toISOString()
    ],
    constructors: [
      new Date(2024, 0, 1).toISOString(),
      new Date(2024, 6, 1).toISOString(),
      new Date(2024, 2, 10, 2, 30).toISOString(),
      new Date(2024, 10, 3, 1, 30).toISOString()
    ],
    parsing: [
      new Date('2024-01-01T00:00:00').toISOString(),
      new Date(Date.parse('2024-01-01T00:00:00')).toISOString(),
      new Date('2024-01-01').toISOString(),
      new Date('2024-01-01T00:00:00+02:00').toISOString()
    ],
    setters: [setterResult, setter.toISOString(), gapSetter.toISOString(), revived.toISOString()],
    functionCallUsesOverride: /GMT-0[45]00/.test(Date()),
    explicitLocalePreserved:
      explicitLocaleDate.toLocaleString('en-US', explicitLocaleOptions) ===
      new Intl.DateTimeFormat('en-US', explicitLocaleOptions).format(explicitLocaleDate),
    reflection: [
      Date.name,
      Date.length,
      typeof Date.UTC,
      typeof Date.parse,
      Object.getOwnPropertyDescriptor(Date, 'UTC').enumerable,
      Intl.NumberFormat.name,
      Intl.NumberFormat.length,
      typeof Intl.NumberFormat.supportedLocalesOf
    ]
  });
})()
"#,
        )
        .expect("native Intl and complete local Date semantics should evaluate");

    assert_eq!(
        result,
        r#"{"intlSubclass":[true,true,true,"fr-FR"],"options":[1,true,"America/New_York"],"dateSubclass":[true,true,true,"2024-01-01T05:00:00.000Z"],"constructors":["2024-01-01T05:00:00.000Z","2024-07-01T04:00:00.000Z","2024-03-10T07:30:00.000Z","2024-11-03T05:30:00.000Z"],"parsing":["2024-01-01T05:00:00.000Z","2024-01-01T05:00:00.000Z","2024-01-01T00:00:00.000Z","2023-12-31T22:00:00.000Z"],"setters":[1704130496789,"2024-01-01T17:34:56.789Z","2024-03-10T07:30:00.000Z","2024-01-01T05:00:00.000Z"],"functionCallUsesOverride":true,"explicitLocalePreserved":true,"reflection":["Date",7,"function","function",false,"NumberFormat",0,"function"]}"#
    );
}
#[test]
fn date_locale_override_updates_reach_existing_main_child_and_isolated_realms() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_parsed_test_vm(
        "https://date-locale-shared-state.test/",
        "<!doctype html><html><body></body></html>",
    );
    vm.eval(
        r#"
const iframe = document.createElement("iframe");
iframe.srcdoc = "<!doctype html><html><body></body></html>";
document.body.appendChild(iframe);
"ready"
"#,
    )
    .expect("child Date/Intl realm setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .expect("child Date/Intl realm should be materialized")
        .context_id;
    let top_isolated_context_id = vm
        .create_isolated_world("date-locale-shared-state", false)
        .expect("top isolated Date/Intl realm should be created");
    const PROBE: &str = r#"
[
  new Intl.DateTimeFormat().resolvedOptions().locale,
  new Intl.DateTimeFormat().resolvedOptions().timeZone,
  new Date(0).getHours()
].join("|")
"#;

    environment.set_locale(Some("fr-FR")).unwrap();
    environment.set_timezone(Some("Asia/Shanghai")).unwrap();
    assert_eq!(vm.eval(PROBE).unwrap(), "fr-FR|Asia/Shanghai|8");
    assert_eq!(
        vm.eval_in_child_default_context(child_context_id, PROBE)
            .unwrap(),
        "fr-FR|Asia/Shanghai|8"
    );
    assert_eq!(
        vm.eval_in_isolated_context(top_isolated_context_id, PROBE)
            .unwrap(),
        "fr-FR|Asia/Shanghai|8"
    );

    environment.set_locale(Some("en-US")).unwrap();
    environment.set_timezone(Some("America/New_York")).unwrap();
    assert_eq!(vm.eval(PROBE).unwrap(), "en-US|America/New_York|19");
    assert_eq!(
        vm.eval_in_child_default_context(child_context_id, PROBE)
            .unwrap(),
        "en-US|America/New_York|19"
    );
    assert_eq!(
        vm.eval_in_isolated_context(top_isolated_context_id, PROBE)
            .unwrap(),
        "en-US|America/New_York|19"
    );
}
#[test]
fn date_locale_methods_are_declared_on_date_prototype() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://date-locale-declared.test/");
    environment.set_timezone(Some("UTC")).unwrap();

    let result = vm
        .eval(
            r#"
(() => {
  const summarize = name => {
    const descriptor = Object.getOwnPropertyDescriptor(Date.prototype, name);
    const value = descriptor && descriptor.value;
    return [
      name,
      !!descriptor,
      descriptor && descriptor.enumerable,
      descriptor && descriptor.writable,
      descriptor && descriptor.configurable,
      typeof value,
      value && value.name,
      value && value.length
    ].join(":");
  };
  const date = new Date(0);
  return [
    Object.keys(Date.prototype).includes("toLocaleString"),
    summarize("toLocaleString"),
    summarize("toLocaleDateString"),
    summarize("toLocaleTimeString"),
    Date.prototype.toLocaleString.call(date)
  ].join("|");
})()
"#,
        )
        .expect("Date locale method descriptor probe should evaluate");

    assert_eq!(
        result,
        "false|toLocaleString:true:false:true:true:function:toLocaleString:0|toLocaleDateString:true:false:true:true:function:toLocaleDateString:0|toLocaleTimeString:true:false:true:true:function:toLocaleTimeString:0|1/1/1970, 12:00:00 AM"
    );
}
