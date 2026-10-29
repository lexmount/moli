globalThis.__mediaCapabilitiesVideoOptionsResult = 'pending';
globalThis.__mediaCapabilitiesVideoOptionsRows = '[]';
(async () => {
    const queries = [
        ['decodingInfo', 'file'],
        ['decodingInfo', 'media-source'],
        ['decodingInfo', 'webrtc'],
        ['encodingInfo', 'record'],
        ['encodingInfo', 'webrtc']
    ];
    const optional = ['hasAlphaChannel', 'hdrMetadataType', 'colorGamut', 'transferFunction', 'scalabilityMode', 'spatialScalability'];
    const variants = [['absent', undefined, undefined]];
    for (const member of optional) variants.push([member + '=undefined', member, undefined]);
    for (const [member, values] of [
        ['hasAlphaChannel', [false, true]],
        ['hdrMetadataType', ['smpteSt2086', 'smpteSt2094-10', 'smpteSt2094-40']],
        ['colorGamut', ['srgb', 'p3', 'rec2020']],
        ['transferFunction', ['srgb', 'pq', 'hlg']],
        ['scalabilityMode', ['L1T1']],
        ['spatialScalability', [false, true]]
    ]) for (const value of values) variants.push([member + '=' + value, member, value]);
    const applicable = {
        hasAlphaChannel: ['decodingInfo/file', 'decodingInfo/media-source', 'decodingInfo/webrtc', 'encodingInfo/record', 'encodingInfo/webrtc'],
        hdrMetadataType: ['decodingInfo/file', 'decodingInfo/media-source'],
        colorGamut: ['decodingInfo/file', 'decodingInfo/media-source'],
        transferFunction: ['decodingInfo/file', 'decodingInfo/media-source'],
        scalabilityMode: ['encodingInfo/webrtc'],
        spatialScalability: ['decodingInfo/file', 'decodingInfo/media-source', 'decodingInfo/webrtc']
    };
    const rows = [];
    for (const [method, type] of queries) {
        const query = method + '/' + type;
        for (const [variant, member, value] of variants) {
            const video = {contentType: type === 'webrtc' ? 'video/VP8' : 'video/webm; codecs="vp09.00.10.08"', width: 320, height: 240, bitrate: 200000, framerate: 24};
            if (member !== undefined) video[member] = value;
            const expectedRejection = value !== undefined && !applicable[member].includes(query);
            let promise = false;
            let error = null;
            try {
                const pending = navigator.mediaCapabilities[method]({type, video});
                promise = pending instanceof Promise;
                await pending;
            } catch (caught) {
                error = caught instanceof TypeError ? 'TypeError' : String(caught);
            }
            rows.push({query, variant, promise, error, pass: promise && (expectedRejection ? error === 'TypeError' : error === null)});
        }
    }
    globalThis.__mediaCapabilitiesVideoOptionsRows = JSON.stringify(rows);
    const failed = rows.filter(row => !row.pass);
    if (failed.length) throw new Error(failed.length + ' option applicability failures; first: ' + failed[0].query + '/' + failed[0].variant);
    return 'ok';
})().then(value => globalThis.__mediaCapabilitiesVideoOptionsResult = value,
          error => globalThis.__mediaCapabilitiesVideoOptionsResult = String(error));
