"""Fixed layout work shared by the mode and Chromium comparison probes."""

PHASES = {
    "clean": """const values=[];let sum=0;for(let j=0;j<8;j++){
      const begin=performance.now();for(let i=0;i<1024;i++)sum+=probe.nodes[i%probe.nodes.length].offsetWidth;
      values.push(performance.now()-begin)}return {sum,timings:values};""",
    "cssom": """let sum=0;const styles=probe.nodes.slice(0,64).map(n=>getComputedStyle(n));
      for(let i=0;i<2048;i++)sum+=styles[i%styles.length].width.length+styles[i%styles.length].color.length;
      return {sum};""",
    "batch": """for(let i=0;i<1024;i++)probe.nodes[i%probe.nodes.length].style.width=(100+i%20)+'px';
      probe.nodes[0].style.width='333px';return {width:probe.nodes[0].offsetWidth};""",
    "alternating": """const values=[],timings=[];for(let i=0;i<64;i++){
      const begin=performance.now();probe.nodes[0].style.width=(300+i%20)+'px';
      values.push(probe.nodes[0].offsetWidth);timings.push(performance.now()-begin)}
      return {values,timings};""",
    "raf_clean": """return new Promise(resolve=>{const stamps=[],widths=[];function frame(t){
      stamps.push(t);widths.push(probe.nodes[0].offsetWidth);
      if(stamps.length===12)resolve({stamps,widths});else requestAnimationFrame(frame)}requestAnimationFrame(frame)});""",
    "raf_write": """return new Promise(resolve=>{const stamps=[],widths=[],timings=[];function frame(t){
      const start=performance.now();probe.nodes[0].style.width=(400+stamps.length)+'px';
      widths.push(probe.nodes[0].offsetWidth);timings.push(performance.now()-start);stamps.push(t);
      if(stamps.length===12)resolve({stamps,widths,timings});else requestAnimationFrame(frame)}requestAnimationFrame(frame)});""",
}


def fixture(nodes, positioned_blocks=False, web_font=False):
    return (
        "<!doctype html><meta charset=utf-8><style>"
        + (
            "@font-face{font-family:LayoutProbe;src:url('/probe.ttf')}"
            if web_font
            else ""
        )
        + "body{margin:0}.item{display:block;"
        "width:100px;height:20px;padding:0;border:0;font:16px "
        + ("LayoutProbe" if web_font else "serif")
        + (";position:relative" if positioned_blocks else "")
        + "}</style>"
        + "<div class=item>Alpha Beta Gamma</div>" * nodes
        + '<div id=scroller style="position:fixed;left:0;top:0;width:200px;height:100px;overflow:auto">'
        '<div style="height:10000px"></div></div><script>window.fixtureReady=true</script>'
    ).encode()
