use anyhow::Result;

use videoson_codec_rav1d::Rav1dSafeDecoder;
use videoson_core::{Demuxer, PixelFormat, VideoDecoder, VideoDecoderOptions};
use videoson_format_ivf::IvfDemuxer;

fn decode_av1_ivf(data: &[u8], threads: Option<usize>) -> Result<Vec<videoson_core::VideoFrame>> {
    let mut demux = IvfDemuxer::new(data.to_vec())?;
    let track = demux.tracks()[0].clone();

    let opts = VideoDecoderOptions {
        threads,
        ..VideoDecoderOptions::default()
    };
    let mut dec = Rav1dSafeDecoder::try_new(&track.codec_params, &opts)?;

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
fn decode_av1_from_ivf_threaded() -> Result<()> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/samples/small_av1.ivf");
    let data = std::fs::read(path)?;
    let frames = decode_av1_ivf(&data, Some(4))?;

    assert_eq!(frames.len(), 5, "expected all 5 AV1 frames");
    for frame in &frames {
        assert_eq!(frame.bit_depth, 8);
        assert_eq!(frame.pixfmt, PixelFormat::Yuv420);
        assert_eq!((frame.width, frame.height), (64, 64));
    }

    let mut demux = IvfDemuxer::new(data)?;
    let track = demux.tracks()[0].clone();
    let opts = VideoDecoderOptions {
        threads: Some(4),
        ..VideoDecoderOptions::default()
    };
    let mut dec = Rav1dSafeDecoder::try_new(&track.codec_params, &opts)?;
    dec.reset()?;
    let mut count = 0;
    while let Some(pkt) = demux.next_packet()? {
        dec.send_packet(&pkt)?;
        while dec.receive_frame()?.is_some() {
            count += 1;
        }
    }
    dec.send_eos()?;
    while dec.receive_frame()?.is_some() {
        count += 1;
    }
    assert_eq!(count, 5, "expected 5 frames after reset with threads");
    Ok(())
}
