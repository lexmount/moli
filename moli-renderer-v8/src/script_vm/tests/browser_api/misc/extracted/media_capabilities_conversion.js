globalThis.__mediaCapabilitiesConversionResult = 'pending';
(async () => {
    const rows = [];
    for (const method of ['decodingInfo', 'encodingInfo']) {
        for (const scenario of ['normal', 'video-before-derived', 'invalid-video-before-type', 'invalid-audio']) {
            let reads = '';
            const videoMarker = {}, keyMarker = {}, typeMarker = {};
            const input = new Proxy({
                audio: scenario === 'invalid-audio' ? null : {contentType: 'audio/webm; codecs="opus"'},
                video: scenario === 'invalid-video-before-type' ? null : {contentType: 'video/webm; codecs="vp09.00.10.08"', width: 320, height: 240, bitrate: 200000, framerate: 24},
                type: scenario === 'invalid-video-before-type' ? undefined : method === 'decodingInfo' ? 'file' : 'webrtc'
            }, {
                get(target, name, receiver) {
                    reads += name + ',';
                    if (scenario === 'video-before-derived') {
                        if (name === 'video') throw videoMarker;
                        if (name === 'keySystemConfiguration') throw keyMarker;
                        if (name === 'type') throw typeMarker;
                    }
                    return Reflect.get(target, name, receiver);
                }
            });
            let outcome = 'resolved', isPromise = false, synchronous = false;
            try {
                let promise;
                try { promise = navigator.mediaCapabilities[method](input); }
                catch (error) { synchronous = true; throw error; }
                isPromise = promise instanceof Promise;
                await promise;
            } catch (error) {
                outcome = error === videoMarker ? 'video' : error === keyMarker ? 'key' : error === typeMarker ? 'type' : error instanceof TypeError ? 'TypeError' : 'other';
            }
            rows.push({method, scenario, reads, outcome, isPromise, synchronous});
        }
    }
    globalThis.__mediaCapabilitiesConversionRows = JSON.stringify(rows);
    const matches = rows.every(row => {
        const reads = row.scenario === 'invalid-audio' ? 'audio,' : row.scenario !== 'normal' ? 'audio,video,' : row.method === 'decodingInfo' ? 'audio,video,keySystemConfiguration,type,' : 'audio,video,type,';
        const outcome = row.scenario === 'normal' ? 'resolved' : row.scenario === 'video-before-derived' ? 'video' : 'TypeError';
        return row.reads === reads && row.outcome === outcome && row.isPromise && !row.synchronous;
    });
    return matches ? 'ok' : JSON.stringify(rows);
})().then(value => globalThis.__mediaCapabilitiesConversionResult = value,
          error => globalThis.__mediaCapabilitiesConversionResult = String(error));
