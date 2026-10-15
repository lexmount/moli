# Video fixtures

`pattern.webm`, `movie_5.webm`, and `counting.webm` are unchanged WPT assets from
snapshot `db95fafd1fcef8428805e41eb5705d444e8c67ce` (`images/pattern.webm`,
`media/movie_5.webm`, and `media/counting.webm`). The renderer uses the same source
bytes for CDP and native integration tests. Their input hashes are kept with the
validation records.

`three-frames-vp9.webm` is generated with libvpx v1.16.0's `vpxenc` from three
16x16 I420 frames. Luma is respectively 16, 128, 235; both chroma planes are 128.
The stream is lossless, one frame per second, with no frame lag or alternate
references. It deliberately carries no color-space tag. Generation options:

```sh
vpxenc --codec=vp9 --webm --lossless=1 --fps=1/1 --passes=1 --limit=3 \
  --width=16 --height=16 --threads=32 --color-space=0 --cpu-used=0 --good \
  --lag-in-frames=0 --auto-alt-ref=0 -o three-frames-vp9.webm input.i420
```

The generation command, encoder version, input sample values and output SHA-256
are recorded in the local `video-frame-source-20261015` validation directory.
