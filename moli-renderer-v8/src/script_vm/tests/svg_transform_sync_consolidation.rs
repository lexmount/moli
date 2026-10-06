use super::*;

#[test]
fn svg_transform_sync_retains_members_and_reflects_windowless_mutations() {
    let mut vm = new_parsed_test_vm(
        "https://svg_transform_sync_consolidation.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
 const docs=[document,document.implementation.createHTMLDocument('detached')];
 for(const doc of docs){
  const group=doc.createElementNS('http://www.w3.org/2000/svg','g');group.setAttribute('transform','translate(1 2)');
  const animated=group.transform,base=animated.baseVal,anim=animated.animVal;
  const baseItem=base.getItem(0),animItem=anim.getItem(0);
  if(group.transform!==animated||group.transform.baseVal!==base||group.transform.animVal!==anim)throw Error('SameObject');
  if(base.getItem(0)!==baseItem||anim.getItem(0)!==animItem)throw Error('unchanged tear-off identity');
  group.setAttribute('transform','translate(3 4)');
  const refreshed=group.transform;
  if(refreshed.baseVal!==base||refreshed.animVal!==anim||base.getItem(0).matrix.e!==3||anim.getItem(0).matrix.f!==4)throw Error('attribute synchronization');
  const item=base.getItem(0);item.setTranslate(5,6);
  if(!group.getAttribute('transform').includes('5'))throw Error('windowless reflection');
  group.transform;
  if(anim.getItem(0).matrix.e!==5||anim.getItem(0).matrix.f!==6)throw Error('animated list synchronization');
  const retained=anim.getItem(0);group.transform;if(anim.getItem(0)!==retained)throw Error('animated tear-off identity');
 }
 return 'ok';
})()"#).expect("native binding regression"), "ok");
}
