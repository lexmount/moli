use super::*;

#[test]
fn hyperlink_protocol_preserves_null_url_fallbacks_across_documents() {
    let mut vm = new_parsed_test_vm(
        "https://hyperlink-null-url-protocol.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
 for(const owner of [document,document.implementation.createHTMLDocument('')]) {
  const base=owner.createElement('base');base.href='about:blank';owner.head.append(base);
  for(const tag of ['a','area']) {
   const link=owner.createElement(tag);
   if(link.protocol!==':')throw Error('missing href '+tag);
   owner.body.append(link);
   for(const href of ['', 'http://[', 'javascript://:443', 'javascript://test:test', 'javascript://[:1]', 'mailto://:443', 'mailto://test:test', 'mailto://[:1]']) {
    link.setAttribute('href',href);
    if(link.protocol!==':'||link.href!==href||link.getAttribute('href')!==href)throw Error('invalid href '+tag+':'+href);
    for(const name of ['host','hostname','port','pathname','search','hash','username','password','origin'])if(link[name]!=='')throw Error('fallback '+name);
   }
   link.href='https://valid.test:8443/path?q#fragment';
   if(link.protocol!=='https:'||link.host!=='valid.test:8443'||link.pathname!=='/path')throw Error('valid href');
  }
 }
 return 'ok';
})()"#).expect("native regression"), "ok");
}
