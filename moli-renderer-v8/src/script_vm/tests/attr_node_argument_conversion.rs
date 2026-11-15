use super::*;

#[test]
fn attribute_node_mutations_use_native_argument_brands_before_bridge_algorithms() {
    let mut vm = new_parsed_test_vm(
        "https://attr-arguments.test/",
        "<!doctype html><body><iframe></iframe>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, label) => { if (!ok) throw Error(label); };
      const other = document.querySelector('iframe').contentWindow;
      const element = document.createElement('div');
      const attr = document.createAttribute('data-name'); attr.value = 'kept';
      for (const realm of [window, other]) {
        for (const name of ['setAttributeNode', 'setAttributeNodeNS', 'removeAttributeNode']) {
          const method = realm.Element.prototype[name];
          let traps = 0;
          const proxy = new Proxy(attr, { get() { traps++; throw Error('author get'); }, getPrototypeOf() { traps++; throw Error('author prototype'); } });
          const revoked = Proxy.revocable(attr, {}); revoked.revoke();
          const invalid = [undefined, null, false, 1, 'attr', {}, Object.create(realm.Attr.prototype), Object.create(attr), proxy, revoked.proxy];
          for (const value of invalid) {
            let error; try { method.call(element, value); } catch (caught) { error = caught; }
            assert(error instanceof realm.TypeError, name + ': argument brand');
            assert(traps === 0 && element.attributes.length === 0 && attr.ownerElement === null, name + ': rejection is atomic');
          }
          let error; try { method.call(element); } catch (caught) { error = caught; }
          assert(error instanceof realm.TypeError, name + ': required argument');
          const badReceiver = new Proxy(element, { get() { traps++; throw Error('receiver trap'); } });
          try { method.call(badReceiver, attr); throw Error('accepted receiver'); } catch (caught) {
            assert(caught instanceof realm.TypeError && traps === 0, name + ': receiver brand');
          }
        }
        assert(realm.Element.prototype.setAttributeNode.call(element, attr) === null, 'genuine Attr accepted');
        assert(element.getAttributeNode('data-name') === attr && element.getAttribute('data-name') === 'kept', 'attached identity');
        assert(realm.Element.prototype.removeAttributeNode.call(element, attr) === attr, 'removed identity');
        assert(element.attributes.length === 0 && attr.ownerElement === null, 'detached after removal');
      }
      return true;
    })()"#).expect("Attr argument conversion probe should evaluate"), "true");
}
