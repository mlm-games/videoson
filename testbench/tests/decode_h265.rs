use anyhow::Result;

use videoson_codec_h265::H265Decoder;
use videoson_core::{
    CodecType, NalFormat, Packet, PixelFormat, PlaneData, VideoCodecParams, VideoDecoder,
    VideoDecoderOptions, VideoFramePlanes,
};

fn plane_u8(frame: &videoson_core::VideoFrame, idx: usize) -> (&[u8], usize) {
    let p = &frame.plane_data[idx];
    match &p.data {
        PlaneData::U8(v) => (v.as_slice(), p.stride),
        _ => panic!("expected U8 plane"),
    }
}

fn plane_u16(frame: &videoson_core::VideoFrame, idx: usize) -> (&[u16], usize) {
    let p = &frame.plane_data[idx];
    match &p.data {
        PlaneData::U16(v) => (v.as_slice(), p.stride),
        _ => panic!("expected U16 plane"),
    }
}

fn read_sample(name: &str) -> Result<Vec<u8>> {
    Ok(std::fs::read(format!(
        "{}/tests/samples/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))?)
}

fn assert_plane_u8(
    frame: &videoson_core::VideoFrame,
    idx: usize,
    w: usize,
    h: usize,
    pattern: fn(usize, usize) -> u8,
) {
    let (plane, stride) = plane_u8(frame, idx);
    assert_eq!(stride, w);
    let expected: Vec<u8> = (0..h)
        .flat_map(|y| (0..w).map(move |x| pattern(x, y)))
        .collect();
    assert_eq!(plane, expected.as_slice());
}

fn assert_plane_u16(
    frame: &videoson_core::VideoFrame,
    idx: usize,
    w: usize,
    h: usize,
    pattern: fn(usize, usize) -> u16,
) {
    let (plane, stride) = plane_u16(frame, idx);
    assert_eq!(stride, w);
    let expected: Vec<u16> = (0..h)
        .flat_map(|y| (0..w).map(move |x| pattern(x, y)))
        .collect();
    assert_eq!(plane, expected.as_slice());
}

fn y8(x: usize, y: usize) -> u8 {
    ((x * 7 + y * 5) & 0xFF) as u8
}

fn cb8(x: usize, y: usize) -> u8 {
    ((x * 3 + y * 11 + 64) & 0xFF) as u8
}

fn cr8(x: usize, y: usize) -> u8 {
    ((x * 13 + y * 17 + 128) & 0xFF) as u8
}

fn y10(x: usize, y: usize) -> u16 {
    ((x * 73 + y * 151) & 0x3FF) as u16
}

fn cb10(x: usize, y: usize) -> u16 {
    ((x * 29 + y * 61 + 512) & 0x3FF) as u16
}

fn cr10(x: usize, y: usize) -> u16 {
    ((x * 97 + y * 37 + 256) & 0x3FF) as u16
}

fn decode_h265_annexb(stream: &[u8]) -> Result<Vec<videoson_core::VideoFrame>> {
    let mut params = VideoCodecParams::new(CodecType::H265);
    params.nal_format = Some(NalFormat::AnnexB);

    let opts = VideoDecoderOptions::default();
    let mut dec = H265Decoder::try_new(&params, &opts)?;

    dec.send_packet(&Packet::new(0, stream.to_vec()))?;
    dec.send_eos()?;

    let mut frames = Vec::new();
    while let Some(f) = dec.receive_frame()? {
        frames.push(f);
    }
    Ok(frames)
}

#[test]
fn decode_h265_tiny_intra() -> Result<()> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/samples/tiny_intra.h265");
    let data = std::fs::read(path)?;
    let frames = decode_h265_annexb(&data)?;
    assert_eq!(frames.len(), 1, "expected 1 frame from tiny_intra");

    let frame = &frames[0];
    assert_eq!(frame.width, 16);
    assert_eq!(frame.height, 16);
    assert_eq!(frame.bit_depth, 8);

    let (y, y_stride) = plane_u8(frame, 0);
    assert_eq!(y.len(), 16 * 16);
    assert_eq!(y_stride, 16);

    Ok(())
}

#[test]
fn decode_h265_inter_p() -> Result<()> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/samples/inter_p.h265");
    let data = std::fs::read(path)?;
    let frames = decode_h265_annexb(&data)?;
    assert!(!frames.is_empty(), "expected at least 1 frame from inter_p");

    for frame in &frames {
        assert_eq!(frame.bit_depth, 8);
        assert!(frame.width > 0);
        assert!(frame.height > 0);
    }

    Ok(())
}

#[test]
fn decode_h265_sao() -> Result<()> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/samples/sao.h265");
    let data = std::fs::read(path)?;
    let frames = decode_h265_annexb(&data)?;
    assert!(!frames.is_empty(), "expected at least 1 frame from sao");

    Ok(())
}

#[test]
fn decode_h265_10bit() -> Result<()> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/samples/10bit_128x128.h265"
    );
    let data = std::fs::read(path)?;
    let frames = decode_h265_annexb(&data)?;
    assert!(!frames.is_empty(), "expected at least 1 frame from 10bit");

    for frame in &frames {
        assert_eq!(frame.bit_depth, 10, "expected 10-bit output");
        assert!(frame.width > 0);
        assert!(frame.height > 0);
    }

    Ok(())
}

#[test]
fn decode_h265_422_8bit() -> Result<()> {
    let frames = decode_h265_annexb(&read_sample("hevc_422_32x32.h265")?)?;
    assert_eq!(frames.len(), 1);
    let f = &frames[0];
    assert_eq!((f.width, f.height, f.bit_depth), (32, 32, 8));
    assert_eq!(f.pixfmt, PixelFormat::Yuv422);
    assert_eq!(f.planes, VideoFramePlanes::Yuv422);
    assert_plane_u8(f, 0, 32, 32, y8);
    assert_plane_u8(f, 1, 16, 32, cb8);
    assert_plane_u8(f, 2, 16, 32, cr8);
    Ok(())
}

#[test]
fn decode_h265_444_8bit() -> Result<()> {
    let frames = decode_h265_annexb(&read_sample("hevc_444_32x32.h265")?)?;
    assert_eq!(frames.len(), 1);
    let f = &frames[0];
    assert_eq!((f.width, f.height, f.bit_depth), (32, 32, 8));
    assert_eq!(f.pixfmt, PixelFormat::Yuv444);
    assert_eq!(f.planes, VideoFramePlanes::Yuv444);
    assert_plane_u8(f, 0, 32, 32, y8);
    assert_plane_u8(f, 1, 32, 32, cb8);
    assert_plane_u8(f, 2, 32, 32, cr8);
    Ok(())
}

#[test]
fn decode_h265_422_10bit() -> Result<()> {
    let frames = decode_h265_annexb(&read_sample("hevc_422_10bit_32x32.h265")?)?;
    assert_eq!(frames.len(), 1);
    let f = &frames[0];
    assert_eq!((f.width, f.height, f.bit_depth), (32, 32, 10));
    assert_eq!(f.pixfmt, PixelFormat::Yuv422);
    assert_eq!(f.planes, VideoFramePlanes::Yuv422);
    assert_plane_u16(f, 0, 32, 32, y10);
    assert_plane_u16(f, 1, 16, 32, cb10);
    assert_plane_u16(f, 2, 16, 32, cr10);
    Ok(())
}

#[test]
fn decode_h265_444_10bit() -> Result<()> {
    let frames = decode_h265_annexb(&read_sample("hevc_444_10bit_32x32.h265")?)?;
    assert_eq!(frames.len(), 1);
    let f = &frames[0];
    assert_eq!((f.width, f.height, f.bit_depth), (32, 32, 10));
    assert_eq!(f.pixfmt, PixelFormat::Yuv444);
    assert_eq!(f.planes, VideoFramePlanes::Yuv444);
    assert_plane_u16(f, 0, 32, 32, y10);
    assert_plane_u16(f, 1, 32, 32, cb10);
    assert_plane_u16(f, 2, 32, 32, cr10);
    Ok(())
}

#[test]
fn decode_h265_crops_conformance_window() -> Result<()> {
    let frames = decode_h265_annexb(&read_sample("hevc_420_window_32x32.h265")?)?;
    assert_eq!(frames.len(), 1);
    let f = &frames[0];
    // 32x32 coded, conf_win right/bottom = 2 chroma units (4 luma each).
    assert_eq!((f.width, f.height, f.bit_depth), (28, 28, 8));
    assert_eq!(f.pixfmt, PixelFormat::Yuv420);
    assert_plane_u8(f, 0, 28, 28, y8);
    assert_plane_u8(f, 1, 14, 14, cb8);
    assert_plane_u8(f, 2, 14, 14, cr8);
    Ok(())
}

/// The threaded path is bit-identical to serial (upstream contract), and a
/// `reset()` rebuilds the decoder without losing the thread budget.
#[test]
fn decode_h265_threads_and_reset_match_serial() -> Result<()> {
    let data = read_sample("inter_p.h265")?;
    let serial = decode_h265_annexb(&data)?;
    assert!(!serial.is_empty());

    let mut params = VideoCodecParams::new(CodecType::H265);
    params.nal_format = Some(NalFormat::AnnexB);
    let opts = VideoDecoderOptions {
        threads: Some(4),
        ..Default::default()
    };
    let mut dec = H265Decoder::try_new(&params, &opts)?;
    dec.send_packet(&Packet::new(0, data.clone()))?;
    dec.reset()?;
    dec.send_packet(&Packet::new(0, data))?;
    dec.send_eos()?;

    let mut frames = Vec::new();
    while let Some(f) = dec.receive_frame()? {
        frames.push(f);
    }

    assert_eq!(serial.len(), frames.len());
    for (a, b) in serial.iter().zip(frames.iter()) {
        assert_eq!(
            (a.width, a.height, a.bit_depth, a.pixfmt),
            (b.width, b.height, b.bit_depth, b.pixfmt)
        );
        for (pa, pb) in a.plane_data.iter().zip(b.plane_data.iter()) {
            match (&pa.data, &pb.data) {
                (PlaneData::U8(x), PlaneData::U8(y)) => assert_eq!(x, y),
                (PlaneData::U16(x), PlaneData::U16(y)) => assert_eq!(x, y),
                _ => panic!("plane type mismatch"),
            }
        }
    }
    Ok(())
}
