use super::*;

#[tokio::test]
async fn worker_offscreen_canvas_transfers_a_bitmap_and_clears_the_canvas() {
    ensure_v8();
    let source = r#"
        const canvas = new OffscreenCanvas(4, 3);
        const ctx = canvas.getContext('2d');
        ctx.fillRect(0, 0, 4, 3);
        const before = ctx.getImageData(1, 1, 1, 1).data[3];
        const bitmap = canvas.transferToImageBitmap();
        postMessage([before, ctx.getImageData(1, 1, 1, 1).data[3], bitmap instanceof ImageBitmap,
            bitmap.width, bitmap.height]);
        close();"#
        .to_string();
    let mut worker = spawn_worker(source, "test://canvas_transfer_to_image_bitmap".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(message), "[255,0,true,4,3]");
}
