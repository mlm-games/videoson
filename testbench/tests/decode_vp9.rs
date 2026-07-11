use anyhow::Result;

use videoson_codec_vp9::Vp9Decoder;
use videoson_core::{
    Demuxer, PixelFormat, VideoDecoder, VideoDecoderOptions,
};
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

    println!("Decoded {} VP9 frames ({}x{})", frames.len(), frames[0].width, frames[0].height);
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
