async function workerCredentiallessFetchProbe(sameOrigin, crossOrigin) {
  const cases = [
    ['same', sameOrigin, 'same', 'no-cors', 'include'],
    ['cross', crossOrigin, 'cross', 'no-cors', 'include'],
    ['cors', crossOrigin, 'cors', 'cors', 'include'],
    ['same-to-cross', sameOrigin, 'same-to-cross', 'no-cors', 'include'],
    ['cross-to-same', crossOrigin, 'cross-to-same', 'no-cors', 'include'],
    ['round-trip', sameOrigin, 'round-trip', 'no-cors', 'include'],
    ['no-corp', crossOrigin, 'no-corp', 'no-cors', 'include'],
    ['same-omit', sameOrigin, 'same-omit', 'no-cors', 'omit'],
    ['cors-omit', crossOrigin, 'cors-omit', 'cors', 'omit'],
  ];
  const results = [];
  for (const [name, origin, path, mode, credentials] of cases) {
    const request = new Request(`${origin}/${path}`, {mode, credentials});
    let result;
    try {
      const response = await fetch(request);
      await response.arrayBuffer();
      result = {type: response.type, status: response.status};
    } catch (error) {
      result = {error: error.name};
    }
    results.push({name, mode: request.mode, credentials: request.credentials, ...result});
  }
  return {isolated: self.crossOriginIsolated, results};
}
