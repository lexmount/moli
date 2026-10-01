use super::*;

#[test]
fn rtc_transport_interface_exposure_interfaces_have_realm_local_constructors_and_exposure() {
    for url in [
        "https://interface-exposure.test/",
        "http://interface-exposure.test/",
    ] {
        let mut vm = new_storage_test_vm(url);
        assert_eq!(vm.eval(r#"
(() => {
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const names = ["RTCCertificate", "RTCDTMFSender", "RTCDtlsTransport", "RTCIceTransport", "RTCRtpSender", "RTCRtpTransceiver", "RTCSctpTransport", "RTCStatsReport"];
  const parents = {"RTCCertificate": null, "RTCDTMFSender": "EventTarget", "RTCDtlsTransport": "EventTarget", "RTCIceTransport": "EventTarget", "RTCRtpSender": null, "RTCRtpTransceiver": null, "RTCSctpTransport": "EventTarget", "RTCStatsReport": null};
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const child = document.createElement('iframe');
  document.body.appendChild(child);
  const other = child.contentWindow;
  for (const realm of [window, other]) {
    for (const name of names) {
      const C = realm[name];
      assert(typeof C === 'function' && C.name === name && C.length === 0, name + ' constructor');
      assert(C.prototype.constructor === C, name + ' prototype identity');
      assert(Object.getPrototypeOf(C.prototype) === (parents[name] ? realm[parents[name]].prototype : realm.Object.prototype), name + ' prototype inheritance');
      assert(realm[name] === C, name + ' stable lazy materialization');
      for (const invoke of [() => C(), () => new C(), () => Reflect.construct(C, [], function Subclass() {})]) {
        let error;
        try { invoke(); } catch (caught) { error = caught; }
        assert(error instanceof realm.TypeError, name + ' rejects construction in callee realm');
      }
      assert(C !== (realm === window ? other[name] : window[name]), name + ' distinct realm constructors');
    }
  }
  return 'passed';
})()
"#).unwrap(), "passed", "{url}");
    }
}
