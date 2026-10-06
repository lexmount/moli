use super::*;

#[test]
fn text_encoder_reports_utf16_progress_at_complete_scalar_boundaries() {
    let mut vm = new_parsed_test_vm(
        "https://text-encoder-utf16-progress.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
 const encoder=new TextEncoder();
 for(const [text,size,read,written] of [['A😀B',0,0,0],['A😀B',1,1,1],['A😀B',4,1,1],['A😀B',5,3,5],['A😀B',6,4,6],['😀',4,2,4],['\ud800',3,1,3]]) {
  const storage=new Uint8Array(size+4).fill(99),dest=storage.subarray(2,2+size);
  const result=encoder.encodeInto(text,dest),expected=encoder.encode(text).slice(0,written);
  if(result.read!==read||result.written!==written||storage[0]!==99||storage[1]!==99||storage.at(-1)!==99)throw Error('progress '+text+':'+size);
  for(let i=0;i<written;i++)if(dest[i]!==expected[i])throw Error('bytes');
  for(let i=written;i<size;i++)if(dest[i]!==99)throw Error('suffix');
 }
 return 'ok';
})()"#).expect("native regression"), "ok");
}
