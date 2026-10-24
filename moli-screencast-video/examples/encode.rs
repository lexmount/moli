use anyhow::{Context, Result};
use moli_image::RgbaImage;
use moli_screencast_video::VideoWriter;
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn main() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .context("usage: encode OUTPUT.mp4")?;
    let start = Instant::now();
    let mut video = VideoWriter::new(Path::new(&path), 319, 239, start)?;
    for (millis, color) in [
        (0, [255, 0, 0, 255]),
        (350, [0, 255, 0, 255]),
        (2500, [0, 0, 255, 255]),
    ] {
        let image = RgbaImage::try_new(319, 239, color.repeat(319 * 239))?;
        video.push_frame(&image, start + Duration::from_millis(millis))?;
    }
    println!("{:?}", video.finish(start + Duration::from_secs(10))?);
    Ok(())
}
