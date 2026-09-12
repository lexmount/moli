// Software-Chromium capability profile, not a media playback test.
(async () => {
  const hd = {width:1920,height:1080,bitrate:5000000,framerate:30};
  const uhd = {width:3840,height:2160,bitrate:20000000,framerate:60};
  const video = (contentType, size=hd, audioType=null) => ({
    type:'file', video:{contentType,...size},
    ...(audioType ? {audio:{contentType:audioType,channels:'2',bitrate:132700,samplerate:5200}} : {})
  });
  const configurations = [
    video('video/mp4; codecs=av01.0.08M.08'),
    video('video/webm; codecs=vp8'),
    video('video/webm; codecs=vp09.00.10.08'),
    video('video/mp4; codecs=hvc1.1.6.L93.B0'),
    video('video/mp4; codecs=avc1.640028'),
    video('video/mp4; codecs=avc1.640033',uhd),
    video('video/webm; codecs=vp09.00.10.08',hd,'audio/ogg; codecs=opus'),
    video('video/mp4; codecs=avc1.640028',hd,'audio/mp4; codecs=mp4a.40.5')
  ];
  const bits = result => Number(result.supported) + 2*Number(result.smooth) + 4*Number(result.powerEfficient);
  const result = await Promise.all(configurations.map(c=>navigator.mediaCapabilities.decodingInfo(c).then(bits)));
  if (JSON.stringify(result) !== '[3,3,3,0,3,3,3,3]') throw new Error(`software profile: ${result}`);
  for (const contentType of ['audio/ogg; codecs=opus','audio/mp4; codecs=mp4a.40.5','audio/flac']) {
    if (bits(await navigator.mediaCapabilities.decodingInfo({type:'file',audio:{contentType}})) !== 7)
      throw new Error(`audio-only profile: ${contentType}`);
  }
  const mseConfigurations = [
    [{...video('video/webm; codecs=vp09.00.10.08'),type:'media-source'},3],
    [{type:'media-source',audio:{contentType:'audio/webm; codecs=opus'}},7],
    [{type:'media-source',audio:{contentType:'audio/ogg; codecs=opus'}},0]
  ];
  for (const [config, expected] of mseConfigurations) {
    if (bits(await navigator.mediaCapabilities.decodingInfo(config)) !== expected)
      throw new Error(`media-source configuration: ${JSON.stringify(config)}`);
  }
  const unsupported = [video('video/mp4'), video('video/webm; codecs=not-a-codec'),
    video('video/webm; codecs=avc1.640028'), video('video/mp4; codecs="avc1.640028,mp4a.40.2"'),
    {type:'file',audio:{contentType:'audio/webm; codecs=vp9'}}];
  for (const config of unsupported) {
    if (bits(await navigator.mediaCapabilities.decodingInfo(config)) !== 0)
      throw new Error(`unsupported configuration: ${JSON.stringify(config)}`);
  }
  for (const config of [{type:'file'}, video('video/webm; unsupported=vp8'),
                        video('video/webm; codecs=vp8',{...hd,framerate:0})]) {
    let rejected = false;
    try { await navigator.mediaCapabilities.decodingInfo(config); }
    catch (error) { rejected = error instanceof TypeError; }
    if (!rejected) throw new Error('invalid configuration must reject with TypeError');
  }
  return JSON.stringify(result);
})()
