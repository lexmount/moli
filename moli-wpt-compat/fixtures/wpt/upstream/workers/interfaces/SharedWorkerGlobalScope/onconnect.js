var results = [];
try {
  self.onconnect = 1;
  results.push(String(onconnect));
} catch(e) {
  results.push(''+e);
}
try {
  // Local correction: EventHandler uses LegacyTreatNonObjectAsNull.
  // Non-callable objects retain their identity; only primitives become null.
  var object = {handleEvent:function(){}};
  self.onconnect = object;
  results.push(onconnect === object);
} catch(e) {
  results.push(''+e);
}
var f = function(e) {
  results.push(e.data);
  e.ports[0].postMessage(results);
};
onconnect = f;
results.push(typeof onconnect);
