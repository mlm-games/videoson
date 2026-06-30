# videoson

A Rust video decode wrapper, with it's workspace design being inspired by Symphonia.

## Workspace Crates

| Crate | Description |
|---|---|
| `videoson-core` | Core traits and types: `VideoDecoder`, `Demuxer`, `Packet`, `VideoFrame`, `CodecRegistry` |
| `videoson-common` | Bitstream utilities: Annex B, avcC, RBSP, Exp-Golomb, BitReader |
| `videoson-format-ivf` | IVF demuxer/header parser |
| `videoson-codec-h264` | H.264 decoder (wraps `rust_h264`, 8-bit YUV420/mono) |
| `videoson-codec-rav1d` | AV1 decoder (wraps `rav1d-safe`, GPLv3) |
| `videoson` | Facade crate with pre-populated registry |

## Status

**Decoding:**
- H.264 (via `rust_h264`, all platforms, 8-bit YUV420 + mono)
- AV1 (via `rav1d-safe`, GPLv3, std-only, 8-bit YUV420)

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
