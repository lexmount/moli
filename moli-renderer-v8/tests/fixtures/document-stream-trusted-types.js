(() => {
const mainRoot = document.documentElement;
const mainBody = document.body;
const strictFrame = mainBody.appendChild(document.createElement('iframe'));
const looseFrame = mainBody.appendChild(document.createElement('iframe'));
const strictWindow = strictFrame.contentWindow;
const strictDocument = strictFrame.contentDocument;
const looseDocument = looseFrame.contentDocument;
const strictWrite = strictWindow.Document.prototype.write;
const strictRoot = strictDocument.documentElement;
const meta = strictDocument.createElement('meta');
meta.httpEquiv = 'Content-Security-Policy';
meta.content = "require-trusted-types-for 'script'";
strictDocument.head.appendChild(meta);
const failures = []; let checks = 0;
function equal(label, actual, expected) { checks++; if (actual !== expected) failures.push({label,actual,expected}); }
try {
 let error;
 try { Document.prototype.write.call(strictDocument, '<p>blocked</p>'); } catch (caught) { error = caught; }
 equal('uses receiver CSP with caller TypeError', error instanceof TypeError, true);
 equal('rejected write preserves receiver', strictDocument.documentElement === strictRoot, true);
 equal('rejected write preserves main', document.body === mainBody, true);
 if (document.documentElement !== mainRoot) { while (document.firstChild) document.firstChild.remove(); document.appendChild(mainRoot); }
 strictWrite.call(looseDocument, '<p>loose</p>');
 Document.prototype.close.call(looseDocument);
 equal('strict-realm method accepts loose receiver', looseDocument.body?.textContent, 'loose');
 equal('borrowed write preserves main', document.body === mainBody, true);
 if (document.documentElement !== mainRoot) { while (document.firstChild) document.firstChild.remove(); document.appendChild(mainRoot); }
 const policyCalls = [];
 strictWindow.trustedTypes.createPolicy('default', { createHTML(input, type, sink) {
   policyCalls.push([type, sink]); return '<p>receiver policy</p>';
 }});
 Document.prototype.write.call(strictDocument, 'input');
 Document.prototype.close.call(strictDocument);
 equal('uses receiver default policy', strictDocument.body?.textContent, 'receiver policy');
 equal('policy arguments', JSON.stringify(policyCalls), JSON.stringify([['TrustedHTML','Document write']]));
 equal('receiver policy write preserves main', document.body === mainBody, true);
} catch (error) { equal('unexpected exception',error.name+': '+error.message,null); }
finally { strictFrame.remove();looseFrame.remove(); if (document.documentElement !== mainRoot) { while (document.firstChild) document.firstChild.remove(); document.appendChild(mainRoot); } }
return {checks,failures};
})()
