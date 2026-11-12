async function observeQuery(call, inspect) {
            let marker = false;
            const observed = call().then(value => [marker, inspect(value)].join(':'),
                error => [marker, error.name].join(':'));
            queueMicrotask(() => { marker = true; });
            return observed;
        }
        async function run() {
            const sw = navigator.serviceWorker;
            const values = await Promise.all([
                observeQuery(() => sw.getRegistration(), value => value.scope === new URL('./', location.href).href),
                observeQuery(() => sw.getRegistrations(), value => Object.isFrozen(value) && value.length === 1),
                observeQuery(() => sw.getRegistration('/unregistered/'), value => value === undefined),
                observeQuery(() => sw.getRegistration('http://['), () => 'unexpected'),
                observeQuery(() => sw.getRegistration('https://different-origin.test/'), () => 'unexpected')
            ]);
            const one = await sw.getRegistration(), all = await sw.getRegistrations();
            values.push(one instanceof ServiceWorkerRegistration && all[0] === one && Object.getPrototypeOf(all) === Array.prototype);
            return values.join('|');
        }
