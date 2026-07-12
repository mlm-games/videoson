# videoson

A Rust video decode wrapper, with its workspace design being inspired by Symphonia.

## Workspace Crates

| Crate | Description |
|---|---|
| `videoson-core` | Core traits and types: `VideoDecoder`, `Demuxer`, `Packet`, `VideoFrame`, `CodecRegistry` |
| `videoson-common` | Bitstream utilities: Annex B, avcC, RBSP, Exp-Golomb, BitReader |
| `videoson-format-ivf` | IVF demuxer/header parser |
| `videoson-codec-*` | decoder crates (wraps crates mentioned below, Apache/MIT) |
| `videoson-codec-rav1d` | AV1 decoder (wraps `rav1d-safe`, **AGPL-3.0-only**) |
| `videoson` | Facade crate with pre-populated registry |

## Status

**Decoding:**
- H.264 (via `rust_h264`, all platforms)
- AV1 (via `rav1d-safe`, AGPL-3.0-or-commercial, std-only, 8-bit YUV420)
- H.265 (via `rust_h265`, all platforms)
- VP8 & VP9 (via `oxideav`, experimental, basic)

**Demuxing:**
- IVF (VP8/VP9/AV1)

## Registry

```rust
use videoson::default_registry;
use videoson_core::{VideoCodecParams, VideoDecoderOptions, CodecType};

let registry = videoson::default_registry();
let params = VideoCodecParams::new(CodecType::H264);
let opts = VideoDecoderOptions::default();
let mut decoder = registry
    .make_video_decoder(&params, &opts)
    .expect("no decoder registered");
```

## Features

| Feature | Dependencies | Default? |
|---|---|---|
| `std` | Standard library support | yes |
| `h264` | H.264 decoder (via `rust_h264`) | yes |
| `h265` | H.265/HEVC decoder (via `rust_h265`) | no |
| `vp8` | VP8 decoder (via `oxideav-vp8`) | no |
| `vp9` | VP9 decoder (via `oxideav-vp9`) | no |
| `rav1d` | AV1 decoder (via `rav1d-safe`, **AGPL-3.0-only**) | no |
| `ivf` | IVF demuxer | yes |
