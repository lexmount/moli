use super::{clean_output, run_fetch_cli_with_dump_and_args, run_moli};
use anyhow::Result;

#[test]
fn realtime_eval_reads_each_current_geometry_without_a_capture() -> Result<()> {
    let output = run_moli([
        "moli",
        "fetch",
        "--realtime",
        "--wait-until",
        "load",
        "--eval",
        "(()=>{const el=document.getElementById('box');const style=getComputedStyle(el);const first=el.offsetWidth;el.style.width='200px';return [first,el.offsetWidth,el.offsetWidth,style.width]})()",
        "data:text/html,<div id=box style='width:100px;height:20px;padding:0;border:0'></div>",
    ])?;
    assert!(output.status.success(), "{}", clean_output(&output.stderr));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value, serde_json::json!([100, 200, 200, "200px"]));
    Ok(())
}

#[test]
fn realtime_fetch_waits_for_raf_resize_and_intersection_business_without_capture() -> Result<()> {
    let html = r#"<!doctype html>
      <div id=box style="width:100px;height:20px"></div>
      <div id=scroller style="width:200px;height:60px;overflow:auto">
        <div style="height:100px"></div><div id=sentinel style="height:10px"></div>
      </div><script>
      window.ro=new ResizeObserver(entries=>{
        if(entries[0].contentRect.width!==200)return;
        ro.disconnect();scroller.scrollTop=100;
      });ro.observe(box);
      window.io=new IntersectionObserver(entries=>{
        if(!entries.some(entry=>entry.isIntersecting))return;
        io.disconnect();const done=document.createElement('output');done.id='ready';
        done.textContent=box.offsetWidth+':'+getComputedStyle(box).width;
        document.body.append(done);
      },{root:scroller});io.observe(sentinel);
      requestAnimationFrame(()=>box.style.width='200px');
      </script>"#;
    let url = format!(
        "data:text/html,{}",
        url::form_urlencoded::byte_serialize(html.as_bytes())
            .collect::<String>()
            .replace('+', "%20")
    );
    let output = run_fetch_cli_with_dump_and_args(
        &url,
        "html",
        &[
            "--realtime",
            "--wait-selector",
            "#ready",
            "--timeout",
            "3000",
        ],
    )?;
    assert!(output.status.success(), "{}", clean_output(&output.stderr));
    let html = clean_output(&output.stdout);
    assert!(
        html.contains("<output id=\"ready\">200:200px</output>"),
        "{html}"
    );
    Ok(())
}

#[test]
fn realtime_fetch_done_does_not_drain_an_infinite_animation_frame_list() -> Result<()> {
    let output = run_moli([
        "moli",
        "fetch",
        "--realtime",
        "--wait-until",
        "done",
        "--timeout",
        "3000",
        "--wait-script",
        "globalThis.framesSeen >= 3",
        "--eval",
        "framesSeen >= 3",
        "data:text/html,<script>window.framesSeen=0;function tick(){framesSeen++;requestAnimationFrame(tick)}requestAnimationFrame(tick)</script>",
    ])?;
    assert!(output.status.success(), "{}", clean_output(&output.stderr));
    assert_eq!(clean_output(&output.stdout).trim(), "true");
    Ok(())
}

#[test]
fn realtime_cli_screenshot_accepts_real_layout_capability() -> Result<()> {
    let output = run_fetch_cli_with_dump_and_args(
        "data:text/html,<style>html,body{margin:0;background:red}</style>",
        "screenshot",
        &[
            "--realtime",
            "--eval",
            "document.documentElement.style.background='blue';document.body.style.background='blue'",
        ],
    )?;
    assert!(output.status.success(), "{}", clean_output(&output.stderr));
    assert!(output.stdout.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(output.stdout.ends_with(b"IEND\xaeB`\x82"));
    Ok(())
}
