globalThis.__mediaCapabilitiesSnapshotResult = 'pending';
(async () => {
    const check = (ok, message) => { if (!ok) throw new Error(message); };
    const capabilities = navigator.mediaCapabilities;
    const audio = () => ({contentType: 'audio/webm; codecs="opus"', channels: '2', bitrate: 128000, samplerate: 48000, spatialRendering: false});
    const video = () => ({contentType: 'video/webm; codecs="vp09.00.10.08"', width: 320, height: 240, bitrate: 200000, framerate: 24, hasAlphaChannel: false, hdrMetadataType: 'smpteSt2086', colorGamut: 'srgb', transferFunction: 'srgb'});
    function dataProperties(value, realm) {
        if (value === null || typeof value !== 'object') return;
        check(Object.getPrototypeOf(value) === (Array.isArray(value) ? realm.Array.prototype : realm.Object.prototype), 'result realm');
        for (const key of Object.keys(value)) {
            const descriptor = Object.getOwnPropertyDescriptor(value, key);
            check(Object.hasOwn(descriptor, 'value') && descriptor.writable && descriptor.enumerable && descriptor.configurable, 'own data property: ' + key);
            dataProperties(descriptor.value, realm);
        }
    }
    for (const method of ['decodingInfo', 'encodingInfo']) {
        let reads = '';
        const child = {};
        for (const [key, value] of Object.entries(audio())) Object.defineProperty(child, key, {configurable: true, get() { reads += key + ','; return value; }});
        const input = {
            get audio() { reads += 'audio,'; return child; },
            get type() { reads += 'type,'; return method === 'decodingInfo' ? 'file' : 'record'; },
            get video() { reads += 'video,'; return undefined; }
        };
        const promise = capabilities[method](input);
        check(promise instanceof Promise, 'query returns a Promise');
        check(reads === 'audio,bitrate,channels,contentType,samplerate,spatialRendering,video,type,', 'base dictionary converts before derived members');
        for (const key of Object.keys(input)) Object.defineProperty(input, key, {get() { throw new Error('input read after conversion'); }});
        for (const key of Object.getOwnPropertyNames(child)) Object.defineProperty(child, key, {get() { throw new Error('nested input read after conversion'); }});
        const result = await promise;
        check(Object.keys(result).join() === (method === 'decodingInfo' ? 'powerEfficient,smooth,supported,configuration,keySystemAccess' : 'powerEfficient,smooth,supported,configuration'), 'result dictionary inheritance order');
        check(result.configuration.audio.channels === '2', 'converted nested result');
        check(Object.keys(result.configuration).join() === 'audio,type', 'configuration output order');
        check(Object.keys(result.configuration.audio).join() === 'bitrate,channels,contentType,samplerate,spatialRendering', 'audio output order');
        dataProperties(result, window);
        const stable = Object.freeze({type: method === 'decodingInfo' ? 'file' : 'record', audio: Object.freeze(audio())});
        const first = await capabilities[method](stable);
        const second = await capabilities[method](stable);
        check(first !== second && first.configuration !== second.configuration && first.configuration.audio !== second.configuration.audio, 'fresh nested output');
        first.configuration.audio.channels = 'changed';
        check(second.configuration.audio.channels === '2' && stable.audio.channels === '2', 'no input or result alias');
        Object.freeze(second.configuration.audio);
        check((await capabilities[method](stable)).configuration.audio.channels === '2', 'frozen prior result');
    }
    for (const scheme of [undefined, null, 'cenc', 'cbc\uD800']) {
        const track = {};
        if (scheme !== undefined) track.encryptionScheme = scheme;
        const input = {type: 'file', audio: audio(), keySystemConfiguration: {keySystem: 'org.w3.clearkey', sessionTypes: ['temporary', 'temporary'], audio: track}};
        const promise = capabilities.decodingInfo(input);
        input.keySystemConfiguration.sessionTypes[0] = 'changed';
        const result = await promise;
        const key = result.configuration.keySystemConfiguration;
        check(Object.keys(result.configuration).join() === 'audio,keySystemConfiguration,type', 'decoding output order');
        check(key.distinctiveIdentifier === 'optional' && key.persistentState === 'optional' && key.initDataType === '', 'dictionary defaults');
        check(key.audio.robustness === '', 'nested dictionary default');
        check(Object.hasOwn(key.audio, 'encryptionScheme'), 'nullable default creates a member');
        check(key.audio.encryptionScheme === (scheme === undefined ? null : scheme), 'nullable member default and value');
        check(Object.keys(key.audio).join() === 'encryptionScheme,robustness', 'nested dictionary member order');
        check(key.sessionTypes.join() === 'temporary,temporary' && key.sessionTypes !== input.keySystemConfiguration.sessionTypes, 'copied sequence preserves duplicates');
        dataProperties(result, window);
    }
    const codeUnits = '\uD800\u0000\uDCFF';
    const unicode = await capabilities.decodingInfo({type: 'file', audio: audio(), keySystemConfiguration: {keySystem: 'org.w3.clearkey', audio: {robustness: codeUnits, encryptionScheme: codeUnits}}});
    check(unicode.configuration.keySystemConfiguration.audio.robustness === codeUnits && unicode.configuration.keySystemConfiguration.audio.encryptionScheme === codeUnits, 'DOMString code units');
    const names = ['audio', 'video', 'type', 'configuration', 'contentType', 'width', 'sessionTypes', 'keySystemConfiguration', 'encryptionScheme'];
    const marker = {};
    for (const mode of ['count', 'throw', 'readonly', 'index']) {
        const input = {type: 'file', audio: audio(), video: video(), keySystemConfiguration: {keySystem: 'org.w3.clearkey', sessionTypes: ['temporary', 'temporary'], audio: {encryptionScheme: null}}};
        const expected = JSON.stringify(input);
        let writes = 0;
        let result;
        try {
            if (mode !== 'index') for (const name of names) Object.defineProperty(Object.prototype, name, {
                configurable: true,
                get() { return undefined; },
                ...(mode === 'readonly' ? {} : {set() { writes++; if (mode === 'throw') throw marker; }})
            });
            if (mode === 'index') Object.defineProperty(Array.prototype, '0', {configurable: true, get() { return undefined; }, set() { writes++; throw marker; }});
            result = await capabilities.decodingInfo(input);
        } finally {
            if (mode !== 'index') for (const name of names) delete Object.prototype[name];
            if (mode === 'index') delete Array.prototype[0];
        }
        check(writes === 0, 'prototype setter invoked: ' + mode);
        check(result.configuration.audio.contentType === input.audio.contentType && result.configuration.video.width === 320, 'pollution cannot remove output');
        check(result.configuration.keySystemConfiguration.sessionTypes.join() === 'temporary,temporary', 'indexed own data');
        check(JSON.stringify(input) === expected, 'input remains unchanged');
        dataProperties(result, window);
    }
    const frame = document.createElement('iframe');
    document.body.append(frame);
    const foreign = frame.contentWindow;
    for (const realm of [window, foreign]) {
        const receiver = realm === window ? foreign.navigator.mediaCapabilities : capabilities;
        const promise = realm.MediaCapabilities.prototype.decodingInfo.call(receiver, {type: 'file', audio: audio()});
        check(Object.getPrototypeOf(promise) === realm.Promise.prototype, 'callee Promise realm');
        const result = await promise;
        dataProperties(result, realm);
        check(result.configuration.audio.channels === '2', 'foreign receiver output');
    }
    frame.remove();
    return 'ok';
})().then(value => globalThis.__mediaCapabilitiesSnapshotResult = value,
          error => globalThis.__mediaCapabilitiesSnapshotResult = String(error));
