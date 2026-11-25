// Evaluate on a blank standards-mode page with CDP Runtime.evaluate.
// Use a 1280x720 viewport and Realtime layout in Moli; the same probe runs
// in Chromium. Timings are diagnostic, while reads, checksums and background
// dimensions must agree between Moli baseline and candidate.
(() => {
  if (document.compatMode !== "CSS1Compat") throw new Error("Standards mode required");
  const config = {
    "fonts": [
      ".Aqua Kana",
      ".Helvetica LT MM",
      ".Times LT MM",
      "sans-serif-thin",
      "ARNO PRO",
      "Agency FB",
      "Arabic Typesetting",
      "Arial Unicode MS",
      "AvantGarde Bk BT",
      "BankGothic Md BT",
      "Batang",
      "Bitstream Vera Sans Mono",
      "Calibri",
      "Century",
      "Century Gothic",
      "Clarendon",
      "EUROSTILE",
      "Franklin Gothic",
      "Futura Bk BT",
      "Futura Md BT",
      "GOTHAM",
      "Gill Sans",
      "HELV",
      "Haettenschweiler",
      "Helvetica Neue",
      "Humanst521 BT",
      "Leelawadee",
      "Letter Gothic",
      "Levenim MT",
      "Lucida Bright",
      "Lucida Sans",
      "Menlo",
      "MS Mincho",
      "MS Outlook",
      "MS Reference Specialty",
      "MS UI Gothic",
      "MT Extra",
      "MYRIAD PRO",
      "Marlett",
      "Meiryo UI",
      "Microsoft Uighur",
      "Microsoft YaHei",
      "Minion Pro",
      "Monotype Corsiva",
      "PMingLiU",
      "Pristina",
      "SCRIPTINA",
      "Segoe UI Light",
      "Serifa",
      "SimHei",
      "Small Fonts",
      "Staccato222 BT",
      "TRAJAN PRO",
      "Univers CE 55 Medium",
      "Vrinda",
      "ZWAdobeF"
    ],
    "bases": [
      "monospace",
      "sans-serif",
      "serif"
    ],
    "text": "mmMwWLliI0O&1",
    "size": "72px"
  };
  document.head.innerHTML = '<style>body{margin:0;font:14px Arial;counter-reset:measurementCount 0}#background{width:1000px}.row{height:18px;padding:2px;box-sizing:border-box;border-bottom:1px solid #ddd}</style>';
  const results=[];
  for (const n of [0,200,800,0]) {
    document.body.replaceChildren();
    const background=document.createElement('main');background.id='background';
    background.innerHTML=Array.from({length:n},(_,i)=>'<div class="row">row '+i+' sample text</div>').join('');
    document.body.appendChild(background);
    const before=background.getBoundingClientRect().width;
    const parent=document.createElement('div');parent.style.cssText='position:absolute;top:-9999px;left:-9999px';
    const span=document.createElement('span');span.style.fontSize=config.size;span.textContent=config.text;
    document.body.appendChild(parent);
    let reads=0,checksum=0;const baseWidth={},baseHeight={};
    const started=performance.now();
    for(const base of config.bases){
      span.style.fontFamily=base;parent.appendChild(span);
      baseWidth[base]=span.offsetWidth;baseHeight[base]=span.offsetHeight;reads+=2;
      parent.removeChild(span);
    }
    const detected=[];
    for(const font of config.fonts){
      let found=false;
      for(const base of config.bases){
        span.style.fontFamily=font+','+base;parent.appendChild(span);
        const width=span.offsetWidth;reads++;checksum+=width;
        let differs=width!==baseWidth[base];
        if(!differs){differs=span.offsetHeight!==baseHeight[base];reads++;}
        found=found||differs;
        parent.removeChild(span);
      }
      if(found)detected.push(font);
    }
    const ms=performance.now()-started;
    console.log('font-probe-case',JSON.stringify({n,ms,reads}));
    parent.remove();
    const after=background.getBoundingClientRect().width;
    results.push({n,ms,reads,checksum,detected,backgroundWidthBefore:before,backgroundWidthAfter:after,nodes:document.querySelectorAll('*').length});
  }
  return {fontCount:config.fonts.length,baseCount:config.bases.length,results};
})()
