(async () => {
    document.head.innerHTML = '<style>html,body{margin:0;padding:0;font:16px monospace;line-height:20px} section {width:200px} img {display:block;margin:0;border:0;padding:0;width:120px;height:auto}</style>';
    document.body.innerHTML = '';
    const cases = [
        ['empty', '80', '40', '', ''],
        ['bad', '80', '40', '', '', true],
        ['alt', '80', '40', 'abc', ''],
        ['css-auto', '80', '40', '', 'aspect-ratio:auto'],
        ['css-square', '80', '40', '', 'aspect-ratio:1/1'],
        ['fraction', '80.5', '40', '', ''],
        ['percent', '80%', '40', '', ''],
        ['relative', '80*', '40', '', ''],
        ['zero', '0', '40', '', ''],
        ['invalid', 'invalid', '40', '', ''],
        ['space', ' 80 ', ' 40 ', '', ''],
        ['suffix', '80px', '40px', '', ''],
    ];
    const loads = [];
    for (const [id, width, height, alt, css, bad] of cases) {
        const section = document.createElement('section');
        const image = document.createElement('img');
        image.id = id;
        image.setAttribute('data-probe', '');
        image.setAttribute('width', width);
        image.setAttribute('height', height);
        image.alt = alt;
        image.style.cssText = css;
        if (bad) {
            loads.push(new Promise(resolve => { image.onload = image.onerror = resolve; }));
            image.src = 'data:image/png;base64,bm90LWEtcG5n';
        }
        section.append(image);
        document.body.append(section);
    }
    await Promise.all(loads);
})()
