use super::*;

#[test]
fn cssom_pending_declarations_survive_mixed_mutations_and_serialization() {
    let mut vm = new_storage_html_test_vm("https://pending-storage.test/");
    let result = vm
        .eval(
            r#"
(() => {
  let checks=0; const failures=[],coverage=[];
  const equal=(actual,expected,label)=>{checks++;if(actual!==expected)failures.push({label,actual,expected});};
  const cases=[
    ['container', 'card / inline-size'], ['columns', '10px 3'],
    ['column-rule', '2px solid red'], ['font-synthesis', 'none'],
    ['text-box', 'trim-both cap alphabetic'], ['text-wrap', 'wrap balance'],
    ['place-items', 'center start'], ['place-self', 'center end'],
    ['inset', '1px 2px 3px 4px'], ['inset-block', '1px 2px'], ['inset-inline', '3px 4px'],
    ['grid-row', '2 / 4'], ['grid-area', '1 / 2 / 3 / 4'],
    ['grid-template', '10px / 20px'], ['grid', '10px / 20px'],
    ['scroll-margin', '1px 2px 3px 4px'], ['scroll-margin-block', '1px 2px'],
    ['scroll-margin-inline', '3px 4px'], ['scroll-padding', '1px 2px 3px 4px'],
    ['scroll-padding-block', '1px 2px'], ['scroll-padding-inline', '3px 4px'],
    ['contain-intrinsic-size', '10px 20px'],
    ['border-block', '2px solid red'], ['border-inline', '3px dotted blue'],
    ['border-block-color', 'red blue'], ['border-inline-color', 'blue green'],
    ['border-block-style', 'solid dashed'], ['border-inline-style', 'dotted double'],
    ['border-block-width', '2px 3px'], ['border-inline-width', '4px 5px'],
    ['border-start-start-radius', '1px 2px'],
    ['padding-block', '1px 2px'], ['padding-inline', '3px 4px'],
    ['animation-range', 'entry 10% exit 90%'],
    ['scroll-timeline', '--progress x'], ['view-timeline', '--progress y'],
    ['background', 'red'], ['border', '1px solid blue'], ['font', 'italic 12px serif'],
    ['animation', '2s linear 1s 3 alternate both paused spin'],
    ['transition', 'opacity 2s ease-in 1s'], ['all', 'initial'],
    ['font-variant', 'small-caps'],
    ['text-decoration', 'underline red']
  ];
  for (const [property] of cases) {
    if(property==='all')continue;
    for(const input of ['var(--pending)','env(test)']) {
      for(const source of ['method','cssText','attribute','detached-document','rule']) {
        for(const priority of ['','important']) {
          const element=(source==='detached-document'?document.implementation.createHTMLDocument(''):document).createElement('div');
          const sheet=new CSSStyleSheet();sheet.replaceSync('div {}');
          const style=source==='rule'?sheet.cssRules[0].style:element.style;
          const label=[property,input,source,priority||'normal'].join(':');
          if(source==='cssText')style.cssText=property+':'+input+(priority?' !important':'')+';';
          else if(source==='attribute')element.setAttribute('style',property+':'+input+(priority?' !important':'')+';');
          else style.setProperty(property,input,priority);
          const verify=(step)=>{
            equal(style.getPropertyValue(property),input,label+':'+step+':value');
            equal(style.getPropertyPriority(property),priority,label+':'+step+':priority');
          };
          verify('initial');
          const side=property==='animation'?'transition-timing-function':'animation-timing-function';
          for(const [name,value] of [['--unrelated','kept'],['opacity','0.5'],[side,'linear(0,1)'],['outline-color','invert']]) {
            style.setProperty(name,value);verify('write '+name);
          }
          const serialized=style.cssText;
          const copy=new CSSStyleSheet();copy.replaceSync('div{'+serialized+'}');
          equal(copy.cssRules[0].style.getPropertyValue(property),input,label+':serialized value');
          equal(copy.cssRules[0].style.getPropertyPriority(property),priority,label+':serialized priority');
          for(const name of [side,'--unrelated','opacity','outline-color']) {
            style.removeProperty(name);verify('remove '+name);
          }
          coverage.push({property,input,source,priority});
        }
      }
    }
  }
  return JSON.stringify({checks,coverage,failures});
})()
"#,
        )
        .expect("pending declaration storage checks should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 17200, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}

#[test]
fn cssom_pending_declarations_survive_compatibility_writes() {
    let mut vm = new_storage_html_test_vm("https://pending-compat-storage.test/");
    let result = vm
        .eval(
            r#"
(() => {
  let checks=0; const failures=[];
  const equal=(actual,expected,label)=>{checks++;if(actual!==expected)failures.push({label,actual,expected});};
  equal(CSS.supports('-webkit-transform-origin','20px 30px 0px'),true,'legacy fixture support');
  for(const [property,input] of [['border','var(--edge) solid black'],['container','var(--name)'],['columns','var(--columns)'],['grid-template','var(--tracks)']]) {
    for(const source of ['method','cssText','attribute','detached-document','rule']) {
      for(const priority of ['','important']) {
        const element=(source==='detached-document'?document.implementation.createHTMLDocument(''):document).createElement('div');
        const sheet=new CSSStyleSheet();sheet.replaceSync('div {}');
        const style=source==='rule'?sheet.cssRules[0].style:element.style;
        const label=[property,source,priority||'normal'].join(':');
        if(source==='cssText')style.cssText=property+':'+input+(priority?' !important':'')+';';
        else if(source==='attribute')element.setAttribute('style',property+':'+input+(priority?' !important':'')+';');
        else style.setProperty(property,input,priority);
        const verify=step=>{equal(style.getPropertyValue(property),input,label+':'+step+':value');equal(style.getPropertyPriority(property),priority,label+':'+step+':priority');};
        verify('initial');
        style.setProperty('-webkit-transform-origin','20px 30px 0px');verify('legacy write');
        equal(style.getPropertyValue('-webkit-transform-origin'),'20px 30px 0px',label+':legacy authored depth');
        style.removeProperty('-webkit-transform-origin');verify('legacy remove');
        style.setProperty('-webkit-transform-origin','initial','important');verify('legacy wide write');
        const copy=new CSSStyleSheet();copy.replaceSync('div{'+style.cssText+'}');
        equal(copy.cssRules[0].style.getPropertyValue(property),input,label+':serialized value');
        equal(copy.cssRules[0].style.getPropertyPriority(property),priority,label+':serialized priority');
      }
    }
  }
  return JSON.stringify({checks,failures});
})()
"#,
        )
        .expect("pending declaration compatibility checks should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 441, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}

#[test]
fn cssom_pending_declarations_survive_rejected_compatibility_token_errors() {
    let mut vm = new_storage_html_test_vm("https://pending-invalid-tokens.test/");
    let result = vm.eval(r#"
(() => {
  let checks = 0; const failures = [];
  const equal = (actual, expected, label) => {checks++; if (actual !== expected) failures.push({label, actual, expected});};
  for (const source of ['method', 'cssText', 'attribute', 'detached-document', 'rule']) {
    for (const value of ['pow())', 'hypot(1))', '1px]', '1px}', '"bad\nstring"']) {
      for (const priority of ['', 'important']) {
        const doc = source === 'detached-document' ? document.implementation.createHTMLDocument('') : document;
        const element = doc.createElement('div'); const sheet = new CSSStyleSheet(); sheet.replaceSync('div {}');
        const style = source === 'rule' ? sheet.cssRules[0].style : element.style;
        const initial = 'border: var(--pending) !important; outline-offset: 12px !important; --retained: marker;';
        if (source === 'attribute') element.setAttribute('style', initial);
        else if (source === 'method') {style.setProperty('border', 'var(--pending)', 'important'); style.setProperty('outline-offset', '12px', 'important'); style.setProperty('--retained', 'marker');}
        else style.cssText = initial;
        const before = style.cssText; const length = style.length;
        style.setProperty('outline-offset', value, priority);
        const label = source + ':' + JSON.stringify(value) + ':' + priority;
        equal(style.getPropertyValue('outline-offset'), '12px', label + ': old value');
        equal(style.getPropertyPriority('outline-offset'), 'important', label + ': old priority');
        equal(style.cssText, before, label + ': unchanged cssText');
        equal(style.length, length, label + ': unchanged length');
        equal(style.getPropertyValue('border'), 'var(--pending)', label + ': pending shorthand');
        equal(style.getPropertyPriority('border'), 'important', label + ': pending priority');
        equal(style.getPropertyValue('border-top-color'), '', label + ': pending longhand');
        equal(style.getPropertyPriority('border-top-color'), 'important', label + ': pending longhand priority');
        equal(style.getPropertyValue('--retained'), 'marker', label + ': custom value');
      }
    }
  }
  return JSON.stringify({checks, failures});
})()
"#).expect("invalid compatibility token checks should execute");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 450, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
