(() => {
    const expected = {
        'empty': ['auto 80 / 40', 60],
        'bad': ['auto 80 / 40', 60],
        'alt': ['auto 80 / 40', 60],
        'css-auto': ['auto', 0],
        'css-square': ['1 / 1', 120],
        'fraction': ['auto 80.5 / 40', 59.625],
        'percent': ['auto', 0],
        'relative': ['auto', 0],
        'zero': ['auto 0 / 40', 0],
        'invalid': ['auto', 0],
        'space': ['auto 80 / 40', 60],
        'suffix': ['auto 80 / 40', 60],
    };
    const failures = [];
    for (const [id, [ratio, height]] of Object.entries(expected)) {
        const image = document.getElementById(id);
        const rect = image.getBoundingClientRect();
        const actualRatio = getComputedStyle(image).aspectRatio;
        if (actualRatio !== ratio || rect.width !== 120 || Math.abs(rect.height - height) > 1/64) {
            failures.push({id, expected: [ratio, 120, height], actual: [actualRatio, rect.width, rect.height]});
        }
        if (image.width !== 120 || image.height !== Math.round(height) || image.naturalWidth !== 0 || image.naturalHeight !== 0) {
            failures.push({id, kind: 'IDL dimensions', expected: [120, Math.round(height), 0, 0], actual: [image.width, image.height, image.naturalWidth, image.naturalHeight]});
        }
    }
    return {cases: Object.keys(expected).length, failures};
})()
