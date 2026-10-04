use anyhow::Result;

use videoson_codec_vp9::Vp9Decoder;
use videoson_core::{Demuxer, PixelFormat, VideoDecoder, VideoDecoderOptions};
use videoson_format_ivf::IvfDemuxer;

fn decode_vp9_ivf(data: &[u8]) -> Result<Vec<videoson_core::VideoFrame>> {
    let mut demux = IvfDemuxer::new(data.to_vec())?;
    let track = demux.tracks()[0].clone();

    let opts = VideoDecoderOptions::default();
    let mut dec = Vp9Decoder::try_new(&track.codec_params, &opts)?;

    let mut frames = Vec::new();
    while let Some(pkt) = demux.next_packet()? {
        dec.send_packet(&pkt)?;
        while let Some(f) = dec.receive_frame()? {
            frames.push(f);
        }
    }
    dec.send_eos()?;
    while let Some(f) = dec.receive_frame()? {
        frames.push(f);
    }
    Ok(frames)
}

#[test]
fn decode_vp9_from_ivf() -> Result<()> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/samples/test_vp9.ivf");
    let data = std::fs::read(path)?;
    let frames = decode_vp9_ivf(&data)?;

    assert!(!frames.is_empty(), "expected at least 1 VP9 frame");

    for frame in &frames {
        assert_eq!(frame.bit_depth, 8);
        assert_eq!(frame.pixfmt, PixelFormat::Yuv420);
        assert!(frame.width > 0);
        assert!(frame.height > 0);
    }

    println!(
        "Decoded {} VP9 frames ({}x{})",
        frames.len(),
        frames[0].width,
        frames[0].height
    );
    Ok(())
}

#[test]
fn decode_vp9_nv12_output() -> Result<()> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/samples/test_vp9.ivf");
    let data = std::fs::read(path)?;
    let mut demux = IvfDemuxer::new(data)?;
    let track = demux.tracks()[0].clone();

    let opts = VideoDecoderOptions {
        output_format: videoson_core::VideoOutputFormat::Nv12,
        ..Default::default()
    };
    let mut dec = Vp9Decoder::try_new(&track.codec_params, &opts)?;

    let mut count = 0;
    while let Some(pkt) = demux.next_packet()? {
        dec.send_packet(&pkt)?;
        while let Some(f) = dec.receive_frame()? {
            assert_eq!(f.pixfmt, PixelFormat::Nv12);
            count += 1;
        }
    }
    dec.send_eos()?;
    while let Some(f) = dec.receive_frame()? {
        assert_eq!(f.pixfmt, PixelFormat::Nv12);
        count += 1;
    }

    assert!(count > 0, "expected at least 1 NV12 frame");
    Ok(())
}

/// Colour has to survive the conversion to a packed output format too. The
/// 8-bit paths build frames through `new_nv12_u8` / `new_yuv420_u8`, which
/// default `color_info`; a decoder that reports colour into one of those and
/// forgets `with_color_info` silently reports nothing.
fn color_of(path: &str) -> Result<videoson_core::ColorInfo> {
    let data = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/samples/").to_owned() + path)?;
    let frames = decode_vp9_ivf(&data)?;
    Ok(frames
        .first()
        .expect("fixture decoded no frames")
        .color_info)
}

#[test]
fn vp9_reports_signalled_bt709_limited() -> Result<()> {
    let c = color_of("small_vp9_bt709_limited.ivf")?;
    assert!(c.signalled);
    assert_eq!((c.primaries, c.transfer, c.matrix), (1, 1, 1));
    assert!(!c.full_range);
    Ok(())
}

#[test]
fn vp9_reports_full_range() -> Result<()> {
    let c = color_of("small_vp9_bt709_full.ivf")?;
    assert!(c.signalled);
    assert_eq!((c.primaries, c.transfer, c.matrix), (1, 1, 1));
    assert!(c.full_range);
    Ok(())
}

/// VP9's CS_BT_601 is H.273 code 6, SMPTE 170M.
#[test]
fn vp9_maps_bt601_to_cicp_6() -> Result<()> {
    let c = color_of("small_vp9_smpte170m_p0.ivf")?;
    assert!(c.signalled);
    assert_eq!((c.primaries, c.transfer, c.matrix), (6, 6, 6));
    Ok(())
}

/// `CS_UNKNOWN` states outright that colour is out-of-band. Reporting it as a
/// concrete code would be inventing data, so nothing must be signalled.
#[test]
fn vp9_leaves_cs_unknown_unsignalled() -> Result<()> {
    let data = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/samples/test_vp9.ivf"))?;
    for f in decode_vp9_ivf(&data)? {
        assert!(!f.color_info.signalled, "CS_UNKNOWN must not be signalled");
        assert_eq!(f.color_info.matrix, videoson_core::CICP_UNSPECIFIED);
    }
    Ok(())
}
