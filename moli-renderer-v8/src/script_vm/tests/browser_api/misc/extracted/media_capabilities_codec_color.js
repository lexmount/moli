globalThis.__mediaCapabilitiesCodecColorResult = 'pending';
globalThis.__mediaCapabilitiesCodecColorRows = '[]';
(async () => {
    const cases = [
        {
                "codec": "vp09.00.10.08",
                "primaries": 1,
                "transfer": 1,
                "malformed": false
        },
        {
                "codec": "vp09.02.10.10.01.09.16.09.01",
                "primaries": 9,
                "transfer": 16,
                "malformed": false
        },
        {
                "codec": "vp09.02.10.10.01.09.18.09.00",
                "primaries": 9,
                "transfer": 18,
                "malformed": false
        },
        {
                "codec": "vp09.01.10.08.03.11.01.01.00",
                "primaries": 11,
                "transfer": 1,
                "malformed": false
        },
        {
                "codec": "vp09.01.10.08.03.12.01.01.00",
                "primaries": 12,
                "transfer": 1,
                "malformed": false
        },
        {
                "codec": "vp09.01.10.08.03.01.13.00.01",
                "primaries": 1,
                "transfer": 13,
                "malformed": false
        },
        {
                "codec": "vp09.00.10.08.01.02.02.02.00",
                "primaries": 2,
                "transfer": 2,
                "malformed": false
        },
        {
                "codec": "vp09.02.10.10.01.02.16.09.00",
                "primaries": 2,
                "transfer": 16,
                "malformed": false
        },
        {
                "codec": "vp09.02.10.10.01.09.02.09.00",
                "primaries": 9,
                "transfer": 2,
                "malformed": false
        },
        {
                "codec": "av01.0.04M.08",
                "primaries": 1,
                "transfer": 1,
                "malformed": false
        },
        {
                "codec": "av01.0.04M.10.0.112.09.16.09.0",
                "primaries": 9,
                "transfer": 16,
                "malformed": false
        },
        {
                "codec": "av01.0.04M.10.0.110.09.18.09.0",
                "primaries": 9,
                "transfer": 18,
                "malformed": false
        },
        {
                "codec": "av01.1.08M.08.0.000.11.01.01.0",
                "primaries": 11,
                "transfer": 1,
                "malformed": false
        },
        {
                "codec": "av01.1.08M.08.0.000.12.01.01.0",
                "primaries": 12,
                "transfer": 1,
                "malformed": false
        },
        {
                "codec": "av01.1.08M.08.0.000.01.13.00.1",
                "primaries": 1,
                "transfer": 13,
                "malformed": false
        },
        {
                "codec": "av01.0.04M.08.0.110.02.02.02.0",
                "primaries": 2,
                "transfer": 2,
                "malformed": false
        },
        {
                "codec": "av01.0.04M.10.0.110.02.16.09.0",
                "primaries": 2,
                "transfer": 16,
                "malformed": false
        },
        {
                "codec": "av01.0.04M.10.0.110.09.02.09.0",
                "primaries": 9,
                "transfer": 2,
                "malformed": false
        },
        {
                "codec": "vp09",
                "malformed": true
        },
        {
                "codec": "vp09.00.10",
                "malformed": true
        },
        {
                "codec": "vp09.0.10.08",
                "malformed": true
        },
        {
                "codec": "vp09.00.1.08",
                "malformed": true
        },
        {
                "codec": "vp09.00.10.8",
                "malformed": true
        },
        {
                "codec": "vp09.04.10.08",
                "malformed": true
        },
        {
                "codec": "vp09.00.99.08",
                "malformed": true
        },
        {
                "codec": "vp09.00.10.09",
                "malformed": true
        },
        {
                "codec": "vp09.00.10.08.01",
                "malformed": true
        },
        {
                "codec": "vp09.00.10.08.01.09.16",
                "malformed": true
        },
        {
                "codec": "vp09.00.10.08.01.09.16.09.02",
                "malformed": true
        },
        {
                "codec": "vp09.00.10.08.01.09.16.09.00.00",
                "malformed": true
        },
        {
                "codec": "vp09.00.10.08.01.09.16.00.00",
                "malformed": true
        },
        {
                "codec": "vp09.-0.10.08",
                "malformed": true
        },
        {
                "codec": "vp09.00.10.08.01.99.16.09.00",
                "malformed": true
        },
        {
                "codec": "vp09.00.10.08.01.09.99.09.00",
                "malformed": true
        },
        {
                "codec": "av01",
                "malformed": true
        },
        {
                "codec": "av01.0.04M",
                "malformed": true
        },
        {
                "codec": "av01.00.04M.08",
                "malformed": true
        },
        {
                "codec": "av01.0.4M.08",
                "malformed": true
        },
        {
                "codec": "av01.0.04m.08",
                "malformed": true
        },
        {
                "codec": "av01.3.04M.08",
                "malformed": true
        },
        {
                "codec": "av01.0.32M.08",
                "malformed": true
        },
        {
                "codec": "av01.0.04M.09",
                "malformed": true
        },
        {
                "codec": "av01.0.04M.08.0",
                "malformed": true
        },
        {
                "codec": "av01.0.04M.08.0.110.09",
                "malformed": true
        },
        {
                "codec": "av01.0.04M.08.0.110.09.16.09.2",
                "malformed": true
        },
        {
                "codec": "av01.0.04M.08.0.110.09.16.09.0.0",
                "malformed": true
        },
        {
                "codec": "av01.0.04M.08.2.110.09.16.09.0",
                "malformed": true
        },
        {
                "codec": "av01.0.04M.08.0.120.09.16.09.0",
                "malformed": true
        },
        {
                "codec": "av01.0.04M.08.0.113.09.16.09.0",
                "malformed": true
        },
        {
                "codec": "av01.0.04M.08.0.100.09.16.09.0",
                "malformed": true
        },
        {
                "codec": "av01.0.04M.08.0.110.99.16.09.0",
                "malformed": true
        },
        {
                "codec": "av01.0.04M.08.0.110.09.99.09.0",
                "malformed": true
        }
];
    const options = [
        {}, {colorGamut: 'srgb'}, {colorGamut: 'p3'}, {colorGamut: 'rec2020'},
        {transferFunction: 'srgb'}, {transferFunction: 'pq'}, {transferFunction: 'hlg'},
        {colorGamut: 'srgb', transferFunction: 'srgb'},
        {colorGamut: 'p3', transferFunction: 'srgb'},
        {colorGamut: 'rec2020', transferFunction: 'pq'},
        {colorGamut: 'rec2020', transferFunction: 'hlg'}
    ];
    const gamuts = {srgb: [1], p3: [11, 12], rec2020: [9]};
    const transfers = {srgb: [1, 13], pq: [16], hlg: [18]};
    const rows = [];
    for (const type of ['file', 'media-source']) {
        for (let index = 0; index < cases.length; index++) {
            const spec = cases[index];
            for (let option = 0; option < options.length; option++) {
                const extra = options[option];
                const expected = !spec.malformed &&
                    (!extra.colorGamut || spec.primaries === 2 || gamuts[extra.colorGamut].includes(spec.primaries)) &&
                    (!extra.transferFunction || spec.transfer === 2 || transfers[extra.transferFunction].includes(spec.transfer));
                const video = {contentType: 'video/webm; codecs="' + spec.codec + '"', width: 320, height: 240, bitrate: 200000, framerate: 24, ...extra};
                let promise = false;
                let supported = null;
                let smooth = null;
                let powerEfficient = null;
                let error = null;
                let snapshot = false;
                try {
                    const pending = navigator.mediaCapabilities.decodingInfo({type, video});
                    promise = pending instanceof Promise;
                    const info = await pending;
                    ({supported, smooth, powerEfficient} = info);
                    const copy = info.configuration?.video;
                    snapshot = copy !== undefined && Object.getPrototypeOf(info) === Object.prototype && copy !== video &&
                        copy.contentType === video.contentType && Object.keys(extra).every(key => copy[key] === extra[key]);
                } catch (caught) {error = caught instanceof TypeError ? 'TypeError' : String(caught);}
                rows.push({type, index, option, promise, error, supported, smooth, powerEfficient, snapshot, expected,
                    pass: promise && error === null && supported === expected && snapshot &&
                        typeof smooth === 'boolean' && typeof powerEfficient === 'boolean' &&
                        (supported || (!smooth && !powerEfficient))});
            }
        }
    }
    globalThis.__mediaCapabilitiesCodecColorRows = JSON.stringify(rows);
    const failed = rows.filter(row => !row.pass);
    if (failed.length) throw new Error(failed.length + ' codec color failures; first: ' + JSON.stringify(failed[0]));
    return 'ok';
})().then(value => globalThis.__mediaCapabilitiesCodecColorResult = value,
          error => globalThis.__mediaCapabilitiesCodecColorResult = String(error));
