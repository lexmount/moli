(function* () {
  const assert=(value,label)=>{if(!value)throw Error(label);};
  document.body.style.cssText='margin:0;width:5000px;height:5000px';
  const box=document.createElement('div');box.style.cssText='width:100px;height:100px;overflow:auto';
  box.innerHTML='<div style="width:1000px;height:900px"></div><div id="scroll-target" style="height:20px"></div><div style="height:1000px"></div>';
  document.body.append(box);const target=box.querySelector('#scroll-target');
  yield;
  for(const object of [window,box]) {
    const position=()=>object===window?[scrollX,scrollY]:[object.scrollLeft,object.scrollTop];
    object.scrollTo(10,20);
    object.scrollTo({get behavior(){object.scrollTo(30,40);return 'instant';}});
    assert(position().join()==='30,40','absolute fallback must use state after conversion');
    object.scrollBy({get behavior(){object.scrollTo(50,60);return 'instant';},left:5,top:7});
    assert(position().join()==='55,67','relative scroll must use state after conversion');
    let failure;
    const failed=object.scrollTo({left:99,get top(){throw 7;}});
    if(object===window) {
      assert(failed instanceof Promise,'Window conversion returns a Promise');
      failed.catch(error=>{failure=error;});
      yield;
      assert(failure===7,'Window conversion rejection identity');
    }
    assert(position().join()==='55,67','failed later member cannot partially scroll');
    object.scrollTo(Infinity,NaN);
    assert(position().join()==='0,0','native nonfinite normalization');
  }
  const positions=[];
  for(const value of [false,true,0,1,'','x',undefined,null]) {
    box.scrollTo(0,0);target.scrollIntoView(value);positions.push(box.scrollTop);
  }
  assert(positions[0]===positions[2] && positions[0]===positions[4],'false, zero, empty string align to end');
  assert(positions[1]===positions[3] && positions[1]===positions[5] && positions[1]===positions[6] && positions[1]===positions[7],'true and nullish align to start');
  assert(positions[0]<positions[1],'boolean union must select different alignments');
  box.remove();return true;
})()
