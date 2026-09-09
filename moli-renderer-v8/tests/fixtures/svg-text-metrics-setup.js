(() => {
  document.head.innerHTML = '<style>text{font:20px serif}.css-hidden{visibility:hidden}.css-none{display:none}</style>';
  document.body.innerHTML = `<svg width="600" height="400">
    <text id="plain" x="10" y="40">AB<tspan id="part">CD</tspan>EF</text>
    <text id="wide" x="10" y="70">WWWW</text>
    <text id="narrow" x="10" y="100">iiii</text>
    <text id="base" x="10" y="130" rotate="0 0 0">ABC</text>
    <text id="moved" x="10" y="160" dx="0 40 20" rotate="0 30 60">ABC</text>
    <text id="spaced" y="190" textLength="200" lengthAdjust="spacing">ABC</text>
    <text id="scaled" y="220" textLength="200" lengthAdjust="spacingAndGlyphs">ABC</text>
    <text id="collapsed" y="250">  A  B  </text>
    <text id="preserved" y="280" xml:space="preserve">  A  B  </text>
    <text id="combining" y="310">e\u0301X</text>
    <text id="astral" y="340">\u{1f600}X</text>
    <text id="empty"></text>
    <text id="hidden" visibility="hidden">AB</text>
    <text id="none" style="display:none">ABC</text>
    <text id="css-hidden" class="css-hidden">ABC</text>
    <text id="css-none" class="css-none">ABC</text>
    <text class="duplicate" id="same">A</text><text class="duplicate" id="same">ABCD</text>
  </svg>`;
})()
