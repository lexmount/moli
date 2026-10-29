window.parseTimeAsyncSawTail = !!document.getElementById("late");
window.parseTimeAsyncSawDcl = window.parseTimeDclFired === true;
// Release the response tail only after observing the parser's partial DOM.
void fetch("/compat/parse-time-async-classic-chunked-executed");
