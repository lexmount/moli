(() => {
    const check = (ok, message) => {if (!ok) throw new Error(message);};
    const cases = [
        ['', 'rgba(0, 0, 0, 0) none repeat scroll 0% 0% / auto padding-box border-box'],
        ['background:blue', 'rgb(0, 0, 255) none repeat scroll 0% 0% / auto padding-box border-box'],
        ['color:red;background:currentcolor', 'rgb(255, 0, 0) none repeat scroll 0% 0% / auto padding-box border-box'],
        ['background:blue content-box', 'rgb(0, 0, 255) none repeat scroll 0% 0% / auto content-box content-box'],
        ['background:blue;background-repeat:no-repeat,repeat', 'rgb(0, 0, 255) none no-repeat scroll 0% 0% / auto padding-box border-box'],
        ['background-image:none,none;background-color:blue', 'none repeat scroll 0% 0% / auto padding-box border-box, rgb(0, 0, 255) none repeat scroll 0% 0% / auto padding-box border-box'],
        ['background-image:none,none;background-size:10px,20px,30px', 'none repeat scroll 0% 0% / 10px auto padding-box border-box, rgba(0, 0, 0, 0) none repeat scroll 0% 0% / 20px auto padding-box border-box'],
        ['background:blue;background-position-x:10%;background-position-y:20%', 'rgb(0, 0, 255) none repeat scroll 10% 20% / auto padding-box border-box'],
        ['background-image:linear-gradient(red,blue),linear-gradient(black,white);background-repeat:no-repeat', 'linear-gradient(rgb(255, 0, 0), rgb(0, 0, 255)) no-repeat scroll 0% 0% / auto padding-box border-box, rgba(0, 0, 0, 0) linear-gradient(rgb(0, 0, 0), rgb(255, 255, 255)) no-repeat scroll 0% 0% / auto padding-box border-box'],
    ];
    for (const [text, expected] of cases) {
        const element = document.createElement('div');
        element.style.cssText = text;
        document.body.append(element);
        const style = getComputedStyle(element);
        check(style.background === expected, text + ': ' + style.background);
        element.remove();
    }
    const element = document.createElement('div');
    element.style.background = 'blue'; document.body.append(element);
    const computed = getComputedStyle(element);
    const previous = computed.background;
    element.style.backgroundColor = 'red';
    check(String(previous).startsWith('rgb(0, 0, 255) '), 'retained value is immutable');
    check(computed.background.startsWith('rgb(255, 0, 0) '), 'retained declaration observes mutations');
    check(computed.backgroundImage === 'none', 'longhand value preserved');
    element.remove();
    check(computed.background === '', 'inactive owner is empty');
    return true;
})()
