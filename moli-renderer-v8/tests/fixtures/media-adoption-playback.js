(async () => {
  const checks=[];
  const progress=globalThis.__mediaAdoptionProgress={stage:'setup',checks};
  const assert=(condition,message)=>{if(!condition)throw Error(message);};
  const wait=milliseconds=>new Promise(resolve=>setTimeout(resolve,milliseconds));
  const realms=[['main',globalThis],['iframe',document.querySelector('iframe').contentWindow]];
  async function until(condition,message) {
    const deadline=Date.now()+5000;
    while(!condition()) {
      assert(Date.now()<deadline,message);
      await wait(10);
    }
  }
  const method=(callee,media,name)=>callee.HTMLMediaElement.prototype[name].call(media);
  async function play(callee,media) {
    let result;
    method(callee,media,'play').then(()=>result={fulfilled:true},error=>result={error});
    await until(()=>result!==undefined,'play Promise did not settle');
    assert(result.fulfilled,'valid media must play: '+String(result.error));
  }
  async function ready(media) {
    await until(()=>media.readyState>=3,'resource did not become ready');
    assert(media.error===null,'valid media must remain error-free');
  }
  async function advances(media) {
    const start=media.currentTime;
    await until(()=>media.currentTime>start+0.02,'playback clock stopped');
  }
  async function frame(media,realm) {
    if(media.localName!=='video')return;
    assert(media.videoWidth===352 && media.videoHeight===288,'decoded natural dimensions');
    const canvas=new realm.OffscreenCanvas(1,1),ctx=canvas.getContext('2d');
    await until(()=>{
      ctx.clearRect(0,0,1,1);ctx.drawImage(media,0,0,1,1);
      return ctx.getImageData(0,0,1,1).data[3]===255;
    },'the adopted video must publish a decoded frame');
  }
  function cleanup(media) {
    media.pause();media.removeAttribute('src');media.load();media.remove();
  }
  const cases=[
    ['loaded playback resumes after adoption',async(owner,callee,target,media)=>{
      await ready(media);await play(callee,media);await advances(media);await frame(media,owner);
      const source=media.currentSrc;let pauses=0;
      media.addEventListener('pause',()=>pauses++);
      target.document.body.appendChild(media);
      assert(media.ownerDocument===target.document && !media.paused,'adoption must preserve playback intent');
      await ready(media);await advances(media);await frame(media,target);
      assert(!media.paused && media.currentSrc===source && pauses===0,'reloading in the new Document must resume without a pause');
    }],
    ['pending playback resumes and settles once',async(owner,callee,target,media)=>{
      let settlements=0,result;
      method(callee,media,'play').then(
        ()=>{settlements++;result={fulfilled:true};},
        error=>{settlements++;result={error};}
      );
      assert(!media.paused,'play changes paused before the Promise settles');
      target.document.body.appendChild(media);
      assert(!media.paused,'moving a pending playback request must preserve playback intent');
      await ready(media);await advances(media);await frame(media,target);
      await until(()=>result!==undefined,'the pre-adoption play Promise stayed pending');
      // A user agent may reload the resource to release frame-specific state.
      assert(result.fulfilled || result.error.name==='AbortError','selection replacement must only abort the old play request');
      assert(settlements===1 && !media.paused,'the old request settles once while playback continues');
    }],
    ['paused media stays paused after adoption',async(owner,callee,target,media)=>{
      await ready(media);assert(media.paused,'loaded media starts paused');
      let plays=0;media.addEventListener('playing',()=>plays++);
      target.document.body.appendChild(media);
      assert(media.paused,'adoption must retain an existing paused state');
      await ready(media);await frame(media,target);
      const position=media.currentTime;await wait(40);
      assert(media.paused && media.currentTime===position && plays===0,'a paused resource must not start playing');
    }],
  ];
  try {
    for(const [ownerName,owner] of realms)for(const [calleeName,callee] of realms)
      for(const kind of ['audio','video'])for(const transport of ['data','http']) {
        const target=owner===realms[0][1]?realms[1][1]:realms[0][1];
        for(const [name,body] of cases) {
          const label=`${ownerName}/${calleeName}/${kind}/${transport}/${name}`;
          progress.stage=label;
          const media=owner.document.createElement(kind);owner.document.body.appendChild(media);
          media.src=__mediaAdoptionSources[transport][kind];method(callee,media,'load');
          try {await body(owner,callee,target,media);checks.push({name:label,passed:true});}
          catch(error){checks.push({name:label,passed:false,detail:String(error)});}
          finally {cleanup(media);}
        }
      }
    progress.stage='one hundred loaded video adoptions';
    const video=document.createElement('video');document.body.appendChild(video);
    globalThis.adoptionBudgetMedia=video;
    video.src=__mediaAdoptionSources.data.video;video.load();
    try {
      await ready(video);await play(globalThis,video);
      for(let index=0;index<100;index++) {
        const target=realms[(index+1)%2][1];
        target.document.body.appendChild(video);
        assert(!video.paused,'repeated adoption must retain playback intent');
        await ready(video);await frame(video,target);
      }
      await advances(video);
      checks.push({name:progress.stage,passed:true});
    } catch(error) {checks.push({name:progress.stage,passed:false,detail:String(error)});}
    finally {cleanup(video);}
  } catch(error) {checks.push({name:'adoption fixture setup',passed:false,detail:String(error)});}
  globalThis.__uiEventResults={checks,total:checks.length};
})()
