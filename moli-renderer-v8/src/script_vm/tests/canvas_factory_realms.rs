use super::*;

#[test]
fn canvas_factories_keep_the_canvas_realm_across_borrowed_calls_and_adoption() {
    let mut vm = new_parsed_test_vm(
        "https://canvas-factory-realms.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
        const assert = (ok,label) => {if (!ok) throw Error(label)};
        const child = document.body.appendChild(document.createElement('iframe')).contentWindow;
        for (const [owner,caller] of [[window,child],[child,window]]) {
            const canvas = owner.document.createElement('canvas');
            caller.document.body.appendChild(canvas);
            const context = caller.HTMLCanvasElement.prototype.getContext.call(canvas,'2d');
            assert(Object.getPrototypeOf(context) === owner.CanvasRenderingContext2D.prototype,'context realm');
            assert(owner.HTMLCanvasElement.prototype.getContext.call(canvas,'2d') === context,'context identity');
            const created = caller.CanvasRenderingContext2D.prototype.createImageData.call(context,1,1);
            const read = caller.CanvasRenderingContext2D.prototype.getImageData.call(context,0,0,1,1);
            for (const data of [created,read]) {
                assert(Object.getPrototypeOf(data) === owner.ImageData.prototype,'ImageData realm');
                assert(Object.getPrototypeOf(data.data) === owner.Uint8ClampedArray.prototype,'pixel array realm');
                assert(data.width === 1 && data.height === 1 && data.data.length === 4,'pixel dimensions');
            }
            const metrics = caller.CanvasRenderingContext2D.prototype.measureText.call(context,'test');
            assert(Object.getPrototypeOf(metrics) === owner.TextMetrics.prototype,'TextMetrics realm');
        }
        return true;
    })()"#).unwrap(),"true");
}

#[test]
fn canvas_2d_creation_uses_native_dimensions_without_author_getters() {
    let mut vm = new_parsed_test_vm(
        "https://canvas-factory-realms.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
        const canvas=document.createElement('canvas');
        canvas.setAttribute('width','4');
        canvas.setAttribute('height','3');
        const reads=[];
        for(const name of ['width','height'])Object.defineProperty(canvas,name,{get(){
            reads.push(name);
            throw Error('author '+name+' getter');
        }});
        const context=canvas.getContext('2d');
        if(!(context instanceof CanvasRenderingContext2D))throw Error('context');
        if(context.canvas!==canvas || canvas.getContext('2d')!==context)throw Error('context identity');
        context.fillStyle='red';
        context.fillRect(3,2,1,1);
        if(String(context.getImageData(3,2,1,1).data)!=='255,0,0,255')throw Error('native bitmap dimensions');
        if(String(context.getImageData(4,3,1,1).data)!=='0,0,0,0')throw Error('outside native bitmap');
        return JSON.stringify(reads);
    })()"#
        )
        .unwrap(),
        "[]"
    );
}
