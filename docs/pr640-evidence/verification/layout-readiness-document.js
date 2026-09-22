(config) => new Promise((resolve,reject) => {
    const started=Date.now();let timer,poll,finished=false;
    const cleanup=()=>{clearTimeout(timer);clearInterval(poll);document.removeEventListener('DOMContentLoaded',onDCL);document.removeEventListener('readystatechange',check);};
    const finish=(error,source)=>{if(finished)return;finished=true;cleanup();if(error)reject(error);else resolve({ready:true,document_lifecycle:source,wait_ms:Date.now()-started});};
    const identity=()=>location.origin===config.origin&&location.pathname===config.pathname&&location.search===config.search;
    const onDCL=()=>{if(!identity())finish(Error('Wrong target document'));else finish(null,'observed DOMContentLoaded');};
    function check(){
        try {
            if(!identity())throw Error('Wrong target document');
            if(document.readyState==='complete'){finish(null,'complete');return;}
            const entries=performance.getEntriesByType('navigation');
            if(entries.length && entries[0].domContentLoadedEventEnd>0)finish(null,'navigation timing DOMContentLoaded completed');
        }catch(error){finish(error);}
    }
    if(!(config.timeoutMs>0&&config.timeoutMs<=30000)){reject(Error('Invalid lifecycle deadline'));return;}
    document.addEventListener('DOMContentLoaded',onDCL);
    document.addEventListener('readystatechange',check);
    timer=setTimeout(()=>finish(Error('Target document readiness timed out')),config.timeoutMs);
    poll=setInterval(check,25);check();
})
