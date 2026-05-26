# videoson

A Rust video demux/decode workspace inspired by Symphonia.

Decoding -> h264 (all platforms) and av1 (wraps rav1d-safe, which makes the lib gpl3) only for now

Demuxing -> mp4 for all platforms, mkv/webm for non-web