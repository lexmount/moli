use super::*;

#[test]
fn update_timing_resolves_effect_state_after_dictionary_getters_transfer_it() {
    let mut vm = new_parsed_test_vm(
        "https://animation-core.test/timing-reentry",
        "<!doctype html><div id=box></div>",
    );
    let result=vm.eval(r#"(() => {
      const box=document.getElementById('box'),effect=new KeyframeEffect(box,[{opacity:0},{opacity:1}],{duration:1000,fill:'both'});
      const first=new Animation(effect);first.pause();first.currentTime=50;let second;
      effect.updateTiming({get duration(){second=new Animation(effect);second.pause();second.currentTime=500;return 2000;}});
      return JSON.stringify([second.effect===effect,first.effect===null,effect.getTiming().duration,
        second.effect.getTiming().duration,getComputedStyle(box).opacity]);
    })()"#).unwrap();
    assert_eq!(result, r#"[true,true,2000,2000,"0.25"]"#);
}

#[test]
fn invalid_timing_after_a_getter_transfers_the_effect_does_not_partially_commit() {
    let mut vm = new_parsed_test_vm(
        "https://animation-core.test/timing-abrupt-transfer",
        "<!doctype html><div id=box></div>",
    );
    let result=vm.eval(r#"(() => {
      const box=document.getElementById('box'),effect=new KeyframeEffect(box,[{opacity:0},{opacity:1}],{duration:1000,fill:'both'});
      const first=new Animation(effect);first.pause();let second,rejected=false;
      try{effect.updateTiming({get duration(){second=new Animation(effect);second.pause();second.currentTime=500;return 2000;},easing:'not-an-easing'});}
      catch(error){rejected=error instanceof TypeError;}
      return JSON.stringify([rejected,second.effect===effect,first.effect===null,effect.getTiming().duration,getComputedStyle(box).opacity]);
    })()"#).unwrap();
    assert_eq!(result, r#"[true,true,true,1000,"0.5"]"#);
}
