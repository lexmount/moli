options => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  if (!document.body) root.appendChild(document.createElement('body'));
  const probe = {
    dialogs: {}, events: [],
    blockKeydown: !!options.blockKeydown,
    blockCancel: !!options.blockCancel,
    state() {
      return {
        open: ['first', 'second'].map(id => this.dialogs[id].open),
        cancels: this.events.filter(event => event.type === 'cancel').map(event => event.id)
      };
    }
  };
  for (const id of options.creationOrder || ['first', 'second']) {
    const dialog = document.createElement('dialog');
    dialog.id = id;
    dialog.innerHTML = '<button type="button">Continue</button>';
    document.body.appendChild(dialog);
    probe.dialogs[id] = dialog;
    for (const type of ['cancel', 'close']) dialog.addEventListener(type, event => {
      probe.events.push({type, id, trusted: event.isTrusted, bubbles: event.bubbles,
        cancelable: event.cancelable});
      if (type === 'cancel' && probe.blockCancel) event.preventDefault();
    });
  }
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape' && probe.blockKeydown) event.preventDefault();
  });
  for (const id of options.openOrder || ['first', 'second']) probe.dialogs[id].showModal();
  if (options.reopen) {
    probe.dialogs[options.reopen].close();
    probe.dialogs[options.reopen].showModal();
  }
  if (options.repeat) probe.dialogs[options.repeat].showModal();
  return probe;
}
