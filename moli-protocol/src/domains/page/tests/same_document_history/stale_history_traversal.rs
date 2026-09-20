use super::*;

const REPEATED_PROBE: &str = r#"(async () => {
 const method=METHOD, child=CHILD;
 let frame,win=window;
 if(child) {
   frame=document.createElement('iframe');frame.src='history.html?repeated';
   const loaded=new Promise(resolve=>frame.onload=resolve);
   document.body.append(frame);await loaded;
   await new Promise(resolve=>setTimeout(resolve,0));
   win=frame.contentWindow;
 }
 const nav=win.navigation,firstKey=nav.currentEntry.key;
 await nav.navigate('#one').finished;
 await nav.navigate('#two').finished;
 if(method==='forward')await nav.traverseTo(firstKey).finished;
 const index=nav.currentEntry.index;
 const destination=nav.entries()[method==='forward'?index+1:method==='back'?index-1:0];
 const events=[];
 nav.addEventListener('navigate',event=>events.push({
   type:event.navigationType,key:event.destination.key===destination.key
 }));
 const begin=()=>method==='traverseTo'?nav.traverseTo(firstKey):nav[method]();
 const a=begin(),b=begin();
 const queuedIndex=nav.currentEntry.index;
 const outcomes=await Promise.all([a.committed,a.finished,b.committed,b.finished].map(p=>
   p.then(entry=>entry.key===destination.key,error=>error.name)));
 const result={outcomes,events,
   sharedCommitted:a.committed===b.committed,
   sharedFinished:a.finished===b.finished,
   unchangedWhileQueued:queuedIndex===index,
   destination:nav.currentEntry.key===destination.key
 };
 if(frame)frame.remove();
 return result;
})()"#;

#[tokio::test(flavor = "multi_thread")]
async fn queued_navigation_api_calls_reuse_the_committed_entry_destination() {
    for child in [false, true] {
        for method in ["back", "forward", "traverseTo"] {
            let mut page = SameDocumentPage::new().await;
            let result = page
                .evaluate(
                    &REPEATED_PROBE
                        .replace("METHOD", &json!(method).to_string())
                        .replace("CHILD", &child.to_string()),
                )
                .await;
            assert_eq!(
                result,
                json!({
                    "outcomes":[true,true,true,true],
                    "events":[{"type":"traverse", "key":true}],
                    "sharedCommitted":true,
                    "sharedFinished":true,
                    "unchangedWhileQueued":true,
                    "destination":true,
                }),
                "{child}/{method}: {result}"
            );
        }
    }
}

const REMOVAL_PROBE: &str = r#"(async () => {
 const method=METHOD, crossDocument=CROSS_DOCUMENT, reinsert=REINSERT;
 const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
 const frame=document.createElement('iframe');
 frame.src='history.html?first';
 let loaded=new Promise(resolve=>frame.onload=resolve);
 document.body.append(frame);await loaded;await sleep(0);
 const firstKey=frame.contentWindow.navigation.currentEntry.key;
 if(crossDocument) {
   loaded=new Promise(resolve=>frame.onload=resolve);
   frame.contentWindow.location.href='history.html?second';
   await loaded;await sleep(0);
 } else {
   await frame.contentWindow.navigation.navigate('#second').finished;
 }
 if(method==='forward') {
   if(crossDocument) {
     loaded=new Promise(resolve=>frame.onload=resolve);
     frame.contentWindow.history.back();await loaded;await sleep(0);
   } else {
     await frame.contentWindow.navigation.back().finished;
   }
 }
 const win=frame.contentWindow,nav=win.navigation;
 const Exception=win.DOMException,PromiseConstructor=win.Promise;
 const events=[],order=[],errors=[{},{}],states=[{},{}];
 for(const type of ['navigate','navigateerror','navigatesuccess','currententrychange']) {
   nav.addEventListener(type,()=>events.push(type));
 }
 for(const type of ['popstate','hashchange'])win.addEventListener(type,()=>events.push(type));
 const results=[0,1].map(()=>method==='traverseTo'?nav.traverseTo(firstKey):nav[method]());
 const promiseRealms=results.every(result=>
   result.committed instanceof PromiseConstructor && result.finished instanceof PromiseConstructor);
 const reactions=results.flatMap((result,index)=>['committed','finished'].map(key=>
   result[key].then(
     ()=>{states[index][key]='fulfilled';order.push(index+':'+key)},
     error=>{states[index][key]=error.name;errors[index][key]=error;order.push(index+':'+key)}
   )
 ));
 frame.remove();
 let replacementLoaded=Promise.resolve();
 if(reinsert) {
   frame.src='history.html?replacement';
   replacementLoaded=new Promise(resolve=>frame.onload=resolve);
   document.body.append(frame);
 }
 await Promise.all(reactions);
 await replacementLoaded;
 await sleep(0);
 const result={
   events,states,promiseRealms,
   errorRealms:errors.every(pair=>['committed','finished'].every(key=>
     pair[key] instanceof Exception && !(pair[key] instanceof DOMException))),
   sameErrors:errors.every(pair=>pair.committed===pair.finished),
   ordered:errors.every((_,index)=>order.indexOf(index+':committed')<order.indexOf(index+':finished')),
   replacement:reinsert?{
     search:frame.contentWindow.location.search,
     entries:frame.contentWindow.navigation.entries().length,
     freshKey:frame.contentWindow.navigation.currentEntry.key!==firstKey
   }:null
 };
 if(reinsert)frame.remove();
 return result;
})()"#;

#[tokio::test(flavor = "multi_thread")]
async fn queued_child_history_traversals_reject_after_removal_without_touching_replacements() {
    for cross_document in [false, true] {
        for method in ["back", "forward", "traverseTo"] {
            for reinsert in [false, true] {
                let mut page = SameDocumentPage::new().await;
                let result = page
                    .evaluate(
                        &REMOVAL_PROBE
                            .replace("METHOD", &json!(method).to_string())
                            .replace("CROSS_DOCUMENT", &cross_document.to_string())
                            .replace("REINSERT", &reinsert.to_string()),
                    )
                    .await;
                assert_eq!(
                    result,
                    json!({
                        "events":[],
                        "states":[
                            {"committed":"AbortError", "finished":"AbortError"},
                            {"committed":"AbortError", "finished":"AbortError"},
                        ],
                        "promiseRealms":true,
                        "errorRealms":true,
                        "sameErrors":true,
                        "ordered":true,
                        "replacement":if reinsert {
                            json!({"search":"?replacement", "entries":1, "freshKey":true})
                        } else {
                            json!(null)
                        },
                    }),
                    "{cross_document}/{method}/{reinsert}: {result}"
                );
            }
        }
    }
}
