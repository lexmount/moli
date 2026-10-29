globalThis.__mediaCapabilitiesMimeResult = 'pending';
globalThis.__mediaCapabilitiesMimeRows = '[]';
(async () => {
    const cases = [
    {
        "kind": "audio",
        "contentType": "audio/webm; codecs=\"opus\"",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/webm",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/ogg; codecs=\"opus\"",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/ogg",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/mp4; codecs=\"opus\"",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/mp4",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/3gpp; codecs=\"opus\"",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/3gpp",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/3gpp2; codecs=\"opus\"",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/3gpp2",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/webm; codecs=\"opus\"; foo=bar",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/webm; foo=bar",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/webm; codecs=\"opus,opus\"",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/webm; codecs=\",opus\"",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/webm; codecs=\"opus,\"",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/webm; codecs=\"\"",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/webm; codecs=\"vp8\"",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/webm; codecs=\"opus\"; codecs=\"vp8\"",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/x-future-media",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "audio",
        "contentType": "audio/x-future-media; codecs=future-codec",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "audio",
        "contentType": "audio/x-future-media; foo=bar",
        "file": false,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "audio",
        "contentType": "application/octet-stream",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "audio",
        "contentType": "application/ogg; codecs=vorbis",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "audio",
        "contentType": "application/ogg; codecs=theora",
        "file": false,
        "rtp": false,
        "unsupportedFile": true
    },
    {
        "kind": "audio",
        "contentType": "application/mp4; codecs=mp4a.40.2",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "audio",
        "contentType": "application/mp4; codecs=avc1.42e01e",
        "file": false,
        "rtp": false,
        "unsupportedFile": true
    },
    {
        "kind": "audio",
        "contentType": "video/webm; codecs=opus",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "text/plain",
        "file": false,
        "rtp": false,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/webm; codecs=\"vp8\"",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/webm",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/ogg; codecs=\"vp8\"",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/ogg",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/mp4; codecs=\"vp8\"",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/mp4",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/3gpp; codecs=\"vp8\"",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/3gpp",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/3gpp2; codecs=\"vp8\"",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/3gpp2",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/webm; codecs=\"vp8\"; foo=bar",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/webm; foo=bar",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/webm; codecs=\"vp8,vp8\"",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/webm; codecs=\",vp8\"",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/webm; codecs=\"vp8,\"",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/webm; codecs=\"\"",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/webm; codecs=\"opus\"",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/webm; codecs=\"vp8\"; codecs=\"opus\"",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/x-future-media",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/x-future-media; codecs=future-codec",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/x-future-media; foo=bar",
        "file": false,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "application/octet-stream",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "application/ogg; codecs=theora",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "application/ogg; codecs=vorbis",
        "file": false,
        "rtp": false,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "application/mp4; codecs=avc1.42e01e",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "application/mp4; codecs=mp4a.40.2",
        "file": false,
        "rtp": false,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "audio/webm; codecs=vp8",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "text/plain",
        "file": false,
        "rtp": false,
        "unsupportedFile": true
    },
    {
        "kind": "audio",
        "contentType": "audio/mpeg",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/mpeg; codecs=opaque",
        "file": false,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/mpeg;",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/mp3",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/mp3; codecs=opaque",
        "file": false,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/mp3;",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/aac",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/aac; codecs=opaque",
        "file": false,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/aac;",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/flac",
        "file": true,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/flac; codecs=opaque",
        "file": false,
        "rtp": true,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/flac;",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "audio",
        "contentType": "audio/opus",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "audio",
        "contentType": "audio/opus; codecs=opaque",
        "file": false,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "audio",
        "contentType": "audio/opus;",
        "file": false,
        "rtp": false,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/VP8",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/VP8; codecs=opaque",
        "file": false,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/VP8;",
        "file": false,
        "rtp": false,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/VP9",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/VP9; codecs=opaque",
        "file": false,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/VP9;",
        "file": false,
        "rtp": false,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/H264",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/H264; codecs=opaque",
        "file": false,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/H264;",
        "file": false,
        "rtp": false,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/AV1",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/AV1; codecs=opaque",
        "file": false,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/AV1;",
        "file": false,
        "rtp": false,
        "unsupportedFile": true
    },
    {
        "kind": "audio",
        "contentType": "audio/opus; minptime=10; useinbandfec=1",
        "file": false,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/VP9; profile-id=0",
        "file": false,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/VP9; profile-level=\"0\"",
        "file": false,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/H264; level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
        "file": false,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/AV1; level-idx=5;profile=0;tier=0",
        "file": false,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/quicktime; codecs=avc1.42e01e",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/quicktime",
        "file": false,
        "rtp": false,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "application/mp21; codecs=avc1.42e01e",
        "file": true,
        "rtp": true,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "application/mp21",
        "file": false,
        "rtp": false,
        "unsupportedFile": true
    },
    {
        "kind": "video",
        "contentType": "video/webm; codecs=\"vp8\u00e9\"",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/webm; codecs=\"vp8 vp9\"",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    },
    {
        "kind": "video",
        "contentType": "video/VP8\"",
        "file": false,
        "rtp": false,
        "unsupportedFile": false
    }
];
    const queries = [['decodingInfo', 'file'], ['decodingInfo', 'media-source'], ['decodingInfo', 'webrtc'], ['encodingInfo', 'record'], ['encodingInfo', 'webrtc']];
    const rows = [];
    for (const [method, type] of queries) {
        for (let index = 0; index < cases.length; index++) {
            const spec = cases[index];
            const valid = type === 'webrtc' ? spec.rtp : spec.file;
            const input = {type, [spec.kind]: spec.kind === 'audio' ? {contentType: spec.contentType} : {contentType: spec.contentType, width: 320, height: 240, bitrate: 200000, framerate: 24}};
            let promise = false;
            let error = null;
            let supported = null;
            let boolean = false;
            try {
                const pending = navigator.mediaCapabilities[method](input);
                promise = pending instanceof Promise;
                const result = await pending;
                supported = result.supported;
                boolean = typeof supported === 'boolean';
            } catch (caught) {error = caught instanceof TypeError ? 'TypeError' : String(caught);}
            const unsupported = type === 'webrtc' || spec.unsupportedFile;
            rows.push({query: method + '/' + type, index, promise, error, supported, pass: promise && (valid ? error === null && boolean && (!unsupported || supported === false) : error === 'TypeError')});
        }
    }
    globalThis.__mediaCapabilitiesMimeRows = JSON.stringify(rows);
    const failed = rows.filter(row => !row.pass);
    if (failed.length) throw new Error(failed.length + ' MIME failures; first: ' + failed[0].query + '/' + failed[0].index);
    return 'ok';
})().then(value => globalThis.__mediaCapabilitiesMimeResult = value,
          error => globalThis.__mediaCapabilitiesMimeResult = String(error));
