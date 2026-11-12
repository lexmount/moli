(() => {
    document.head.innerHTML = `<style>
      html,body {margin:0;padding:0;background:white}
      svg {display:block}
      .projected #shape {fill:red;stroke:blue;stroke-width:2px}
      .projected #label {
        font-family:"Missing & Family",serif;font-size:32px;font-weight:700;
        fill:green !important;text-anchor:middle;letter-spacing:2px
      }
      .projected #span {font-family:monospace;font-size:20px;font-style:italic;fill:blue}
    </style>`;
    document.body.innerHTML = `<svg id="svg" width="300" height="100" viewBox="0 0 300 100">
      <rect id="shape" x="2" y="2" width="24" height="20" fill="red" stroke="blue" stroke-width="2"/>
      <text id="label" x="150" y="70" font-family="&quot;Missing &amp; Family&quot;,serif"
            font-size="32" font-weight="700" fill="green" text-anchor="middle" letter-spacing="2">Wi<tspan
            id="span" font-family="monospace" font-size="20" font-style="italic" fill="blue">iiWW</tspan>W</text>
    </svg>`;
    globalThis.enableSvgDocumentStyles = () => {
        for (const [id, names] of [
            ['shape', ['fill', 'stroke', 'stroke-width']],
            ['label', ['font-family', 'font-size', 'font-weight', 'fill', 'text-anchor', 'letter-spacing']],
            ['span', ['font-family', 'font-size', 'font-style', 'fill']],
        ]) {
            const element = document.getElementById(id);
            for (const name of names) element.removeAttribute(name);
        }
        // A trailing unfinished CSS comment must not swallow the sampled style.
        document.getElementById('label').setAttribute('style', 'fill:purple;/*');
        document.getElementById('svg').classList.add('projected');
    };
    return 'installed';
})()
