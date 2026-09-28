(() => {
    'use strict';
    const check = (ok, message) => {if (!ok) throw new Error(message);};
    const eq = (a,b,message) => check(Object.is(a,b),message);
    const throws = (fn,C) => {let caught;try{fn();}catch(e){caught=e;}check(caught instanceof C,`expected ${C.name}`);return caught;};
    const parse = CSSNumericValue.parse;
    const desc = Object.getOwnPropertyDescriptor(CSSNumericValue,'parse');
    check(desc.enumerable && desc.configurable && desc.writable,'descriptor');
    eq(parse.length,1,'arity');
    eq(parse.name,'parse','name');
    throws(()=>parse(),TypeError);
    throws(()=>parse(Symbol()),TypeError);
    throws(()=>new parse('1px'),TypeError);
    for (const invalid of [
        '', ' ', 'auto', '1 2', '1px;', '1px !important', '1xyz', '1number', '1percent',
        '(1px)', 'var(--x)', 'env(foo)', 'url(1px)', 'calc(var(--x))',
        'calc()', 'min()', 'min(1px,)', 'max(,1px)', 'clamp(1px,2px)', 'clamp(1px,2px,3px,4px)',
        'calc(1px + 2s)', 'min(1px,0s)', 'calc(calc(1px * 2s) + 3%)',
        'calc(1px+ 2px)', 'calc(1px +2px)', 'calc(1px/**/+ 2px)', 'calc(1px +/**/2px)',
        'calc(1px))', 'calc(-(1px))', 'calc(1px, 2px)', 'calc([1px])',
    ]) eq(throws(()=>parse(invalid),DOMException).name,'SyntaxError',invalid);
    for (const source of [' 1px ', '1PX', '1\\70 x', '1px/**/']) {
        const v=parse(source);check(v instanceof CSSUnitValue && v.value===1 && v.unit==='px',source);
    }
    for (const source of ['calc(1px + 2px)', 'calc(1px /**/+ 2px)', 'calc(1px + /**/ 2px)', 'calc(1px/**/ + 2px)', 'calc(1px\n+\t2px)']) {
        check(parse(source).equals(new CSSMathSum(CSS.px(3))),source);
    }
    check(parse('calc(10px)').equals(new CSSMathSum(CSS.px(10))),'calc wrapper');
    check(parse('calc(1px + 1in)').equals(new CSSMathSum(CSS.px(97))),'canonicalize lengths');
    check(parse('calc(calc(1px + 2em) + 3rem)').equals(new CSSMathSum(CSS.px(1),CSS.em(2),CSS.rem(3))),'flatten nested sums');
    check(parse('calc(1em + 2px + 3em)').equals(new CSSMathSum(CSS.em(4),CSS.px(2))),'combine same units');
    check(parse('calc(1px - 2 * 3em)').equals(new CSSMathSum(CSS.px(1),new CSSMathNegate(CSS.em(6)))),'precedence and subtraction');
    check(parse('calc((1px + 2em) * 3)').equals(new CSSMathSum(CSS.px(3),CSS.em(6))),'distribute multiplication');
    check(parse('calc((1px + 2em) / 2)').equals(new CSSMathSum(CSS.px(.5),CSS.em(1))),'distribute division');
    check(parse('min(1px,2em)').equals(new CSSMathMin(CSS.px(1),CSS.em(2))),'unresolved minimum');
    check(parse('max(1px,2em)').equals(new CSSMathMax(CSS.px(1),CSS.em(2))),'unresolved maximum');
    check(parse('clamp(1px,2%,3px)').equals(new CSSMathClamp(CSS.px(1),CSS.percent(2),CSS.px(3))),'unresolved clamp');
    eq(parse('min(1in,2px)').value,2,'resolved minimum');
    eq(parse('clamp(20px,0px,10px)').value,20,'lower clamp wins');
    check(parse('calc(min(1px,2em))') instanceof CSSMathMin,'no redundant nested wrapper');
    eq(parse('16777217px').value,16777217,'literal f64');
    eq(parse('calc(16777217px + 2px)').to('px').value,16777219,'expression f64');
    eq(parse('0.12345678901234567%').value,0.12345678901234567,'percentage f64');
    eq(parse('-0').value,-0,'literal negative zero');
    eq(parse('calc(-0)').values[0].value,-0,'math negative zero');
    eq(parse('1e999px').value,3.4028234663852886e38,'CSS token numeric range');
    eq(parse('calc(1px / 0)').values[0].value,Infinity,'CSS division by zero');
    check(Number.isNaN(parse('calc(0 / 0)').values[0].value),'CSS zero over zero');
    eq(parse('calc(pi)').values[0].value,Math.PI,'pi');
    eq(parse('calc(e)').values[0].value,Math.E,'e');
    eq(parse('calc(-infinity)').values[0].value,-Infinity,'negative infinity');
    check(parse('calc(1px').equals(new CSSMathSum(CSS.px(1))),'CSS EOF closes function');
    eq(parse('1x').unit,'x','resolution alias token');
    eq(parse('1x').to('dppx').value,1,'resolution alias conversion');
    eq(new CSSUnitValue(2,'x').type().resolution,1,'resolution alias type');
    eq(new CSSUnitValue(2,'x').to('dpi').value,192,'resolution alias constructor');
    const realms=[globalThis,document.querySelector('iframe').contentWindow];
    for(const realm of realms) {
        const method=realm.CSSNumericValue.parse;
        const U=realm.CSSUnitValue,S=realm.CSSMathSum;
        const sentinel={};let calls=0;
        let caught;try{method({toString(){calls++;throw sentinel;}});}catch(e){caught=e;}
        eq(caught,sentinel,'conversion exception identity');eq(calls,1,'one USVString conversion');
        eq(throws(()=>method('invalid'),realm.DOMException).name,'SyntaxError','exception realm');
        throws(()=>method(),realm.TypeError);
        const revoked=Proxy.revocable({},{});revoked.revoke();
        for(const receiver of [null,{},revoked.proxy]) check(method.call(receiver,'1px') instanceof U,'static method accepts arbitrary this');
        const result=method('calc(1px + 2em)');
        check(result instanceof S && result.values[0] instanceof U && result.values[1] instanceof U,'callee realm for entire graph');
        realm.CSSUnitValue=realm.CSSMathSum=function(){throw sentinel;};
        try {check(method('calc(1px)') instanceof S && method('1px') instanceof U,'intrinsic allocation');}
        finally {realm.CSSUnitValue=U;realm.CSSMathSum=S;}
        let ignored=0;check(method('1px',{toString(){ignored++;throw sentinel;}}) instanceof U,'ignore surplus arguments');eq(ignored,0,'no surplus conversion');
        const value=method({toString(){calls++;return 'calc(2px + 3px)';}});
        eq(calls,2,'single successful conversion');eq(value.to('px').value,5,'converted source');
    }
    return true;
})()
