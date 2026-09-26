use super::*;
use crate::util::v8str;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain, data_properties)]
struct NullableRequiredDictionaryValueProbe<'scope> {
    value: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct NullableRequiredDictionaryAbsentProbe {}

fn assert_body_utf8_bom_probe(scenario: &str) {
    let mut vm = new_storage_test_vm("https://body-utf8.test/");
    let script = r#"
globalThis.__bodyUtf8Result = 'pending';
async function probeBodyUtf8(scenario) {
  const encode = value => new TextEncoder().encode(value);
  const make = (source, bytes, type = 'text/plain;charset=UTF-16') => {
    const headers = {'Content-Type': type};
    if (source === 'request') {
      return new Request('https://body-utf8.test/', {method: 'POST', body: bytes, headers});
    }
    if (source === 'response') return new Response(bytes, {headers});
    let offset = 0;
    const stream = new ReadableStream({async pull(controller) {
      await Promise.resolve();
      if (offset === bytes.length) controller.close();
      else controller.enqueue(bytes.slice(offset, ++offset));
    }});
    return new Response(stream, {headers});
  };
  const check = (value, label) => {if (!value) throw new Error(label);};
  for (const source of ['request', 'response', 'response-stream']) {
    if (scenario === 'text') {
      const cases = [
        [encode('\uFEFFA中'), 'A中'],
        [encode('\uFEFF\uFEFFx'), '\uFEFFx'],
        [encode('x\uFEFF'), 'x\uFEFF'],
        [encode('\uFEFF'), ''],
        [new Uint8Array([0xEF]), '\uFFFD'],
        [new Uint8Array([0xEF, 0xBB]), '\uFFFD'],
        [new Uint8Array([0xEF, 0xBB, 0x41]), '\uFFFDA'],
        [new Uint8Array([0xFF, 0xFE, 0x41, 0]), '\uFFFD\uFFFDA\0'],
        [new Uint8Array([0xEF, 0xBB, 0xBF, 0xF0, 0x9F, 0x41]), '\uFFFDA'],
        [encode('A中'), 'A中'],
        [new Uint8Array(), ''],
      ];
      for (const [index, [bytes, expected]] of cases.entries()) {
        const body = make(source, bytes);
        check(await body.text() === expected, source + '/text/' + index);
        check(body.bodyUsed, source + '/text/' + index + ': bodyUsed');
      }
    } else if (scenario === 'json') {
      const cases = [
        [encode('\uFEFF{"value":1}'), {value: 1}],
        [encode('\uFEFF"\uFEFFvalue"'), '\uFEFFvalue'],
        [encode('\uFEFF\uFEFF{}'), undefined],
        [encode(' \uFEFF{}'), undefined],
        [encode('\uFEFF'), undefined],
        [new Uint8Array([0xFF, 0xFE, 0x7B, 0, 0x7D, 0]), undefined],
        [new Uint8Array([0xEF, 0xBB, 0xBF, 0x22, 0xFF, 0x22]), '\uFFFD'],
        [encode('0'), 0],
      ];
      for (const [index, [bytes, expected]] of cases.entries()) {
        const body = make(source, bytes);
        let value, error;
        try {value = await body.json();} catch (caught) {error = caught;}
        if (expected === undefined) {
          check(error instanceof SyntaxError, source + '/json/' + index + ': SyntaxError');
        } else {
          check(!error && JSON.stringify(value) === JSON.stringify(expected), source + '/json/' + index);
        }
        check(body.bodyUsed, source + '/json/' + index + ': bodyUsed');
      }
    } else {
      const bytes = encode('\uFEFF\uFEFFvalue\uFEFF');
      for (const method of ['bytes', 'arrayBuffer', 'blob', 'stream']) {
        const body = make(source, bytes);
        let actual;
        if (method === 'stream') {
          const reader = body.body.getReader();
          const values = [];
          for (;;) {
            const {done, value} = await reader.read();
            if (done) break;
            values.push(...value);
          }
          actual = new Uint8Array(values);
        } else {
          const value = await body[method]();
          actual = method === 'bytes' ? value : new Uint8Array(method === 'blob' ? await value.arrayBuffer() : value);
        }
        check(actual.length === bytes.length && actual.every((value, index) => value === bytes[index]),
          source + '/' + method + ': raw BOM bytes');
      }
      const fields = await make(source, encode('\uFEFFname=\uFEFFvalue'),
        'application/x-www-form-urlencoded').formData();
      check(fields.get('\uFEFFname') === '\uFEFFvalue', source + ': form values');
    }
  }
  return 'ok';
}
"#;
    vm.eval(&format!(
        "{script}\nprobeBodyUtf8({scenario:?}).then(value => __bodyUtf8Result = value, error => __bodyUtf8Result = String(error));"
    ))
    .expect("Body UTF-8 probe should schedule");
    vm.eval("0").expect("Body UTF-8 promises should drain");
    assert_eq!(
        vm.eval("__bodyUtf8Result")
            .expect("Body UTF-8 probe should finish"),
        "ok",
        "{scenario}"
    );
}

mod byte_streams_and_observers;
mod readable_streams_and_byob;
mod transform_and_readable_streams;
mod url_and_initializer_semantics;
mod webidl_and_transform_streams;
mod webidl_interfaces_and_url_params;
