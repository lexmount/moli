(() => {
  document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
img{display:block;width:40px;height:30px}
#edges,#borderbox{padding:2px;border:3px solid red}
#borderbox{box-sizing:border-box}
#transformed{transform:scale(2)}
#zoomed{zoom:2}
#vertical{writing-mode:vertical-rl}
#fractional{width:40.5px;height:30.5px}
#hidden{display:none}
</style>`;
  document.body.innerHTML = `
<img id=css><img id=override width=90 height=80>
<img id=edges><img id=borderbox><img id=transformed>
<img id=zoomed><img id=vertical><img id=fractional>
<img id=hidden width=33 height=22>`;
  return 'installed';
})()
