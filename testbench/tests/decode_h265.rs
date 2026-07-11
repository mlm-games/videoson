use anyhow::Result;

use videoson_codec_h265::H265Decoder;
use videoson_core::{
    CodecType, NalFormat, Packet, PlaneData, VideoCodecParams, VideoDecoder, VideoDecoderOptions,
};

fn plane_u8(frame: &videoson_core::VideoFrame, idx: usize) -> (&[u8], usize) {
    let p = &frame.plane_data[idx];
    match &p.data {
        PlaneData::U8(v) => (v.as_slice(), p.stride),
        _ => panic!("expected U8 plane"),
    }
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
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/samples/10bit_128x128.h265");
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
