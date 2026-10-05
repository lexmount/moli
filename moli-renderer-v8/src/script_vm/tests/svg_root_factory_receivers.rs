use super::*;

#[test]
fn svg_root_factories_validate_native_receivers_in_main_and_child_realms() {
    let mut vm = new_parsed_test_vm(
        "https://svg-root-receivers.test/",
        "<!doctype html><html><body><iframe></iframe></body></html>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const ns = 'http://www.w3.org/2000/svg';
      const child = document.querySelector('iframe').contentWindow;
      const check = (ok, label) => { if (!ok) throw Error(label); };
      const error = f => { try { f(); } catch (e) { return e; } };
      for (const realm of [window, child]) {
        for (const doc of [realm.document, realm.document.implementation.createHTMLDocument('')]) {
          const svg = doc.createElementNS(ns, 'svg');
          const matrix = svg.createSVGMatrix();
          check(matrix instanceof realm.SVGMatrix && matrix.a === 1 && matrix.d === 1, 'genuine matrix factory');
          const transform = svg.createSVGTransform();
          check(transform instanceof realm.SVGTransform && transform.matrix.a === 1, 'genuine transform factory');
          matrix.a = 2;
          matrix.e = 5;
          const fromMatrix = svg.createSVGTransformFromMatrix(matrix);
          check(fromMatrix instanceof realm.SVGTransform && fromMatrix.matrix.a === 2 && fromMatrix.matrix.e === 5, 'matrix conversion');
          matrix.a = 4;
          check(fromMatrix.matrix.a === 2, 'matrix conversion returns independent state');
          let traps = 0;
          const handler = {get() { traps++; throw Error('receiver trap'); }, getPrototypeOf() { traps++; throw Error('prototype trap'); }};
          const revoked = realm.Proxy.revocable(svg, handler);
          revoked.revoke();
          const receivers = [{}, Object.create(svg), new realm.Proxy(svg, handler), revoked.proxy, doc, doc.createElementNS(ns, 'rect')];
          for (const method of ['createSVGMatrix', 'createSVGTransform', 'createSVGTransformFromMatrix', 'createSVGRect', 'deselectAll']) {
            const fn = realm.SVGSVGElement.prototype[method];
            for (const receiver of receivers) {
              check(error(() => fn.call(receiver, matrix)) instanceof realm.TypeError, method + ' invalid receiver');
            }
          }
          check(traps === 0, 'receiver branding does not invoke author Proxy traps');
        }
      }
      const otherSvg = child.document.createElementNS(ns, 'svg');
      check(SVGSVGElement.prototype.createSVGMatrix.call(otherSvg) instanceof SVGMatrix, 'cross-realm genuine root');
      check(SVGSVGElement.prototype.createSVGTransform.call(otherSvg) instanceof SVGTransform, 'callee-realm transform');
      return true;
    })()"#).unwrap(), "true");
}
