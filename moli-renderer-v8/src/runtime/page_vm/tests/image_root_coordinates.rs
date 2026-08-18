use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_exposes_image_coordinates_in_untransformed_root_space() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/image-root-coordinates.html")?,
        );
        page_vm
            .vm_mut()
            .set_layout_policy(crate::real_layout_test_policy());
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
img{display:block;width:1px;height:1px}
.container{height:100px;width:100px}
#x4-zoom-container{zoom:4}
#ancestor-transform{position:absolute;left:220px;top:20px;transform:translate(40px,30px)}
#ancestor-transform img{width:10px;height:10px}
</style>`;
document.body.innerHTML = `
<div class=container>
  <img id=no-zoom>
  <img id=x2-zoom style="zoom:2">
</div>
<div class=container id=x4-zoom-container>
  <img id=x4-relative style="position:relative;top:10px">
  <img id=x8-effective style="zoom:2">
  <img id=transformed style="transform:scale(5)">
</div>
<div id=ancestor-transform><img id=ancestor-transformed></div>
<img id=negative style="position:absolute;left:-10.75px;top:-20.75px">
<img id=hidden style="display:none">
`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(480, 320, 1.0))?
            .expect("image-coordinate fixture must retain a layout root");

        let result = page_vm.vm_mut().eval(
            r#"JSON.stringify((()=>{
const ids=['no-zoom','x2-zoom','x4-relative','x8-effective','transformed','ancestor-transformed','negative'];
const geometry=Object.fromEntries(ids.map(id=>{
  const image=document.getElementById(id);
  const rect=image.getBoundingClientRect();
  return [id,[image.x,image.y,rect.left,rect.top]];
}));
const xDescriptor=Object.getOwnPropertyDescriptor(HTMLImageElement.prototype,'x');
const yDescriptor=Object.getOwnPropertyDescriptor(HTMLImageElement.prototype,'y');
const noZoom=document.getElementById('no-zoom');
let wrongReceiver;
try{xDescriptor.get.call(document.createElement('div'))}catch(error){wrongReceiver=error.name}
const detached=document.createElement('img');
return {
  geometry,
  hidden:[hidden.x,hidden.y],
  detached:[detached.x,detached.y],
  descriptors:[
    typeof xDescriptor.get,xDescriptor.get.name,xDescriptor.get.length,xDescriptor.set,
    xDescriptor.enumerable,xDescriptor.configurable,
    typeof yDescriptor.get,yDescriptor.get.name,yDescriptor.get.length,yDescriptor.set,
    yDescriptor.enumerable,yDescriptor.configurable,
    Object.hasOwn(noZoom,'x'),Object.hasOwn(noZoom,'y')
  ],
  wrongReceiver
};
})())"#,
        )?;
        let result: serde_json::Value = serde_json::from_str(&result)?;

        for (id, expected) in [
            ("no-zoom", [8, 8, 8, 8]),
            ("x2-zoom", [8, 9, 8, 9]),
            ("x4-relative", [8, 148, 8, 148]),
            ("x8-effective", [8, 112, 8, 112]),
        ] {
            assert_eq!(result["geometry"][id], serde_json::json!(expected), "{id}");
        }
        let transformed = result["geometry"]["transformed"]
            .as_array()
            .expect("transformed image geometry");
        assert_eq!(transformed[0], serde_json::json!(8));
        assert_eq!(transformed[1], serde_json::json!(120));
        assert_ne!(
            transformed[0], transformed[2],
            "image x ignores the image's own CSS transform"
        );
        assert_ne!(
            transformed[1], transformed[3],
            "image y ignores the image's own CSS transform"
        );
        assert_eq!(
            result["geometry"]["ancestor-transformed"],
            serde_json::json!([220, 20, 260, 50]),
            "only getBoundingClientRect includes the ancestor transform"
        );
        let negative = result["geometry"]["negative"]
            .as_array()
            .expect("negative image geometry");
        assert_eq!(negative[0], serde_json::json!(-10));
        assert_eq!(negative[1], serde_json::json!(-20));
        assert_eq!(result["hidden"], serde_json::json!([0, 0]));
        assert_eq!(result["detached"], serde_json::json!([0, 0]));
        assert_eq!(
            result["descriptors"],
            serde_json::json!([
                "function", "get x", 0, null, true, true,
                "function", "get y", 0, null, true, true,
                false, false
            ])
        );
        assert_eq!(result["wrongReceiver"], "TypeError");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("image root-coordinate fixture should run");
}
