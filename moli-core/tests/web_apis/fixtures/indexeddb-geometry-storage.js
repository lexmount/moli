globalThis.geometryStorageProbe = async function(name) {
  const checks = globalThis.geometryStorageChecks = [];
  const check = (label, pass) => checks.push({label, pass: !!pass});
  const request = r => new Promise((resolve,reject) => {r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});
  const done = tx => new Promise((resolve,reject) => {tx.oncomplete=resolve;tx.onabort=()=>reject(tx.error);});
  const names=['DOMPoint','DOMPointReadOnly','DOMRect','DOMRectReadOnly','DOMQuad','DOMMatrix','DOMMatrixReadOnly'];
  const saved=names.map(key=>Object.getOwnPropertyDescriptor(globalThis,key));
  const pointFields=['x','y','z','w'], rectFields=['x','y','width','height'];
  const matrixFields=Array.from({length:16},(_,i)=>'m'+(Math.floor(i/4)+1)+(i%4+1));
  const pointValues=[-0,Infinity,-Infinity,NaN],rectValues=[1,-0,Infinity,NaN];
  const quadValues=[[1,2,3,4],[-0,Infinity,NaN,1],[5,6,7,8],[9,10,11,12]];
  const items=[new DOMPoint(...pointValues),new DOMPointReadOnly(...pointValues),new DOMRect(...rectValues),new DOMRectReadOnly(...rectValues),new DOMQuad(...quadValues.map(v=>new DOMPoint(...v))),new DOMMatrix([1,2,3,4,5,6]),DOMMatrixReadOnly.fromMatrix({is2D:false})];
  const quadPoints=['p1','p2','p3','p4'].map(key=>items[4][key]);
  const prototypes=items.map(item=>Object.getPrototypeOf(item));
  const pointGetters=pointFields.map(key=>Object.getOwnPropertyDescriptor(DOMPointReadOnly.prototype,key).get);
  const rectGetters=rectFields.map(key=>Object.getOwnPropertyDescriptor(DOMRectReadOnly.prototype,key).get);
  const quadGetters=['p1','p2','p3','p4'].map(key=>Object.getOwnPropertyDescriptor(DOMQuad.prototype,key).get);
  const matrixGetters=matrixFields.map(key=>Object.getOwnPropertyDescriptor(DOMMatrixReadOnly.prototype,key).get);
  const dimensionGetter=Object.getOwnPropertyDescriptor(DOMMatrixReadOnly.prototype,'is2D').get;
  const matrixValues=items.slice(5).map(item=>matrixGetters.map(get=>get.call(item)));
  let authorReads=0, db;
  const poison={configurable:true,get(){authorReads++;throw Error('author geometry accessor');}};
  const opening=indexedDB.open(name,1);opening.onupgradeneeded=()=>opening.result.createObjectStore('values');
  db=await request(opening);
  try {
    const graph={firstPoint:quadPoints[0],items,repeated:items[4],lastPoint:quadPoints[3]};graph.self=graph;
    for(let i=0;i<items.length;i++) {
      items[i].expando='excluded';
      const fields=i<2?pointFields:i<4?rectFields:i===4?['p1','p2','p3','p4']:[...matrixFields,'is2D'];
      for(const key of [...fields,'toJSON']) Object.defineProperty(items[i],key,poison);
    }
    for(const point of quadPoints) for(const key of [...pointFields,'toJSON']) Object.defineProperty(point,key,poison);
    const tx=db.transaction('values','readwrite'),complete=done(tx);
    tx.objectStore('values').put(graph,'geometry');
    Object.getOwnPropertyDescriptor(DOMPoint.prototype,'x').set.call(items[0],99);
    Object.getOwnPropertyDescriptor(DOMRect.prototype,'width').set.call(items[2],99);
    Object.getOwnPropertyDescriptor(DOMPoint.prototype,'x').set.call(quadPoints[0],99);
    Object.getOwnPropertyDescriptor(DOMMatrix.prototype,'m11').set.call(items[5],99);
    await complete;
    db.close();db=await request(indexedDB.open(name));
    for(const key of names) Object.defineProperty(globalThis,key,poison);
    const copy=await request(db.transaction('values').objectStore('values').get('geometry'));
    check('graph cycle',copy.self===copy);check('graph alias',copy.repeated===copy.items[4]);check('new graph',copy.items!==items);
    check('point reference before Quad',copy.firstPoint===quadGetters[0].call(copy.items[4]));
    check('point reference after Quad',copy.lastPoint===quadGetters[3].call(copy.items[4]));
    for(let i=0;i<items.length;i++) {
      const item=copy.items[i];
      check('native prototype '+names[i],Object.getPrototypeOf(item)===prototypes[i]);
      check('no expando '+names[i],item.expando===undefined);check('independent object '+names[i],item!==items[i]);
      if(i<4) {
        const getters=i<2?pointGetters:rectGetters,expected=i<2?pointValues:rectValues;
        getters.forEach((get,j)=>check('native scalar '+i+'/'+j,Object.is(get.call(item),expected[j])));
      } else if(i===4) {
        quadGetters.forEach((get,j)=>pointGetters.forEach((coordinate,k)=>check('native quad '+j+'/'+k,Object.is(coordinate.call(get.call(item)),quadValues[j][k]))));
      } else {
        matrixGetters.forEach((get,j)=>check('native matrix '+i+'/'+j,Object.is(get.call(item),matrixValues[i-5][j])));
        check('matrix dimension state '+i,dimensionGetter.call(item)===(i===5));
      }
    }
    check('author accessors not consulted',authorReads===0);
    return {state:checks.every(c=>c.pass)?'pass':'fail',checks};
  } finally {
    names.forEach((key,i)=>Object.defineProperty(globalThis,key,saved[i]));
    db.close();await request(indexedDB.deleteDatabase(name));
  }
};
