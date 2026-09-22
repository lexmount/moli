/* Read-only readiness contract. No click, value assignment, reload, or module loading.
   The caller fixes origin/pathname from the original navigation action. */
(config) => new Promise((resolve, reject) => {
    const started = Date.now();
    const timeoutMs = config.timeoutMs;
    if (!Number.isFinite(timeoutMs) || timeoutMs <= 0 || timeoutMs > 30000 ||
        !config.origin || !config.pathname || !config.targetValue) {
        reject(new Error('Invalid bounded readiness configuration'));
        return;
    }
    let last = 'modules';
    const check = () => {
        try {
            if (location.origin !== config.origin || location.pathname !== config.pathname) {
                throw new Error('Readiness reached a different document');
            }
            const req = window.requirejs;
            if (typeof req === 'function' && typeof req.defined === 'function' &&
                req.defined('ko')) {
                // Synchronous access only after defined(): never initiate AMD loading.
                const ko = req('ko');
                if (!ko || typeof ko.contextFor !== 'function' || !ko.bindingProvider ||
                    !ko.bindingProvider.instance || typeof ko.isObservable !== 'function') {
                    throw new Error('Loaded binding modules lack required observation APIs');
                }
                const expand = document.querySelector('button[data-action="grid-filter-expand"]');
                const apply = document.querySelector('button[data-action="grid-filter-apply"]');
                const select = document.querySelector('select[name="status"]');
                last = 'filter DOM';
                if (expand && apply && select &&
                    [...select.options].some(option => option.value === config.targetValue)) {
                    const style = getComputedStyle(expand);
                    const rect = expand.getBoundingClientRect();
                    const interactive = expand.isConnected && !expand.disabled &&
                        !expand.matches(':disabled') && expand.getAttribute('aria-disabled') !== 'true' &&
                        style.display !== 'none' && style.visibility !== 'hidden' &&
                        style.visibility !== 'collapse' && rect.width > 0 && rect.height > 0;
                    last = 'first expand control';
                    if (interactive) {
                        const ec = ko.contextFor(expand), ac = ko.contextFor(apply), sc = ko.contextFor(select);
                        last = 'Knockout contexts';
                        if (ec && ac && sc) {
                            const provider = ko.bindingProvider.instance;
                            const bindings = node => typeof provider.getBindingAccessors === 'function'
                                ? provider.getBindingAccessors(node, ko.contextFor(node))
                                : null;
                            const eb = bindings(expand), ab = bindings(apply);
                            const component = ac.$data;
                            last = 'component binding contract';
                            if (component && typeof component.apply === 'function' &&
                                ec.$data === component && ec.$collapsible &&
                                ko.isObservable(ec.$collapsible.opened) &&
                                ko.bindingHandlers && ko.bindingHandlers.toggleCollapsible &&
                                typeof ko.bindingHandlers.toggleCollapsible.init === 'function' &&
                                ko.bindingHandlers.click && typeof ko.bindingHandlers.click.init === 'function' &&
                                eb && typeof eb.toggleCollapsible === 'function' &&
                                ab && typeof ab.click === 'function' && ab.click() === component.apply) {
                                resolve({ready: true, target: 'first Filters control',
                                    controls_present: true, target_option_present: true,
                                    expand_interactive: true, knockout_component_contexts: true,
                                    wait_ms: Date.now() - started});
                                return;
                            }
                        }
                    }
                }
            }
            if (Date.now() - started >= timeoutMs) {
                throw new Error('Filters readiness timed out at ' + last);
            }
            setTimeout(check, 25);
        } catch (error) {
            reject(error);
        }
    };
    check();
})
