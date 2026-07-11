# videoson

A Rust video decode wrapper, with it's workspace design being inspired by Symphonia.

## Workspace Crates

| Crate | Description |
|---|---|
| `videoson-core` | Core traits and types: `VideoDecoder`, `Demuxer`, `Packet`, `VideoFrame`, `CodecRegistry` |
| `videoson-common` | Bitstream utilities: Annex B, avcC, RBSP, Exp-Golomb, BitReader |
| `videoson-format-ivf` | IVF demuxer/header parser |
| `videoson-codec-*` | decoder crates (wraps crates mentioned below, Apache/MIT) |
| `videoson-codec-rav1d` | AV1 decoder (wraps `rav1d-safe`, GPLv3) |
| `videoson` | Facade crate with pre-populated registry |

## Status

**Decoding:**
- H.264 (via `rust_h264`, all platforms)
- AV1 (via `rav1d-safe`, GPLv3, std-only, 8-bit YUV420)
- H.265 (via `rust_h265`, all platforms)
- VP8 & VP9 (via `oxideav`, experimental, basic)

**Demuxing:**
- IVF (VP8/VP9/AV1)

## Registry

```rust
use videoson::default_registry;

let registry = videoson::default_registry();
let mut decoder = registry
    .make_video_decoder(&params, &opts)?
    .expect("no decoder registered");
```
