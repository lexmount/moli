document.head.innerHTML = `<style>
html,body { margin:0; background:white }
#editor { width:80px; white-space:pre; font:24px/32px monospace; caret-color:red; outline:none }
#atomic { display:inline-block; width:24px; height:24px; background:lime }
</style>`;
document.body.innerHTML = '<div id="editor" contenteditable></div>';
window.setAtomicCaret = (text, offset, styles = '') => {
  const editor = document.getElementById('editor');
  editor.style.cssText = styles;
  editor.innerHTML = '<span id="atomic" contenteditable="false"></span>';
  editor.append(document.createTextNode(text));
  editor.blur();
  editor.focus();
  getSelection().collapse(editor.lastChild, offset);
};
setAtomicCaret('\n', 0);
'installed';
