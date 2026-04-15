use anyhow::Result;

use less_avc::LessEncoder;

use videoson_codec_h264::H264Decoder;
use videoson_core::{
    CodecType, NalFormat, Packet, PlaneData, VideoCodecParams, VideoDecoder, VideoDecoderOptions,
};

use testbench::{gen_mono12, gen_mono8, gen_yuv4208, OwnedImage};

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

fn decode_all_h264_annexb(stream: Vec<u8>) -> Result<Vec<videoson_core::VideoFrame>> {
    let mut params = VideoCodecParams::new(CodecType::H264);
    params.nal_format = Some(NalFormat::AnnexB);

    let opts = VideoDecoderOptions::default();
    let mut dec = H264Decoder::try_new(&params, &opts)?;

    dec.send_packet(&Packet::new(0, stream))?;

    let mut frames = Vec::new();
    while let Some(f) = dec.receive_frame()? {
        frames.push(f);
    }
    Ok(frames)
}

fn encode_two_frames(img1: &OwnedImage, img2: &OwnedImage) -> Result<Vec<u8>> {
    let (initial, mut enc) = LessEncoder::new(&img1.view())?;

    let mut stream = Vec::new();
    for nal in initial.into_iter() {
        stream.extend_from_slice(&nal.to_annex_b_data());
    }

    let nal2 = enc.encode(&img2.view())?;
    stream.extend_from_slice(&nal2.to_annex_b_data());

    Ok(stream)
}

#[test]
fn decode_less_avc_yuv420_8bit_two_frames() -> Result<()> {
    let img1 = gen_yuv4208(32, 32, false)?;
    let img2 = gen_yuv4208(32, 32, true)?;

    let stream = encode_two_frames(&img1, &img2)?;
    let frames = decode_all_h264_annexb(stream)?;
    assert_eq!(frames.len(), 2);

    for (i, (img, frame)) in [&img1, &img2].into_iter().zip(frames.iter()).enumerate() {
        assert_eq!(frame.width, img.width());
        assert_eq!(frame.height, img.height());
        assert_eq!(frame.bit_depth, 8, "frame {i}");

        let y_ref = img.y_visible_u8().unwrap();
        let (y, y_stride) = plane_u8(frame, 0);
        assert_eq!(y_stride, img.width() as usize);

        assert_eq!(y.len(), (img.width() * img.height()) as usize);
        assert_eq!(y, y_ref.as_slice(), "Y mismatch frame {i}");

        let (u_ref, v_ref) = img.uv_visible_u8().unwrap();
        let (u, _) = plane_u8(frame, 1);
        let (v, _) = plane_u8(frame, 2);

        assert_eq!(u, u_ref.as_slice(), "U mismatch frame {i}");
        assert_eq!(v, v_ref.as_slice(), "V mismatch frame {i}");
    }

    Ok(())
}

#[test]
fn decode_less_avc_mono8_cropped_odd_dims() -> Result<()> {
    let img1 = gen_mono8(15, 14, false)?;
    let img2 = gen_mono8(15, 14, true)?;

    let stream = encode_two_frames(&img1, &img2)?;
    let frames = decode_all_h264_annexb(stream)?;
    assert_eq!(frames.len(), 2);

    for (i, (img, frame)) in [&img1, &img2].into_iter().zip(frames.iter()).enumerate() {
        assert_eq!(frame.width, img.width());
        assert_eq!(frame.height, img.height());
        assert_eq!(frame.bit_depth, 8, "frame {i}");

        let y_ref = img.y_visible_u8().unwrap();
        let (y, y_stride) = plane_u8(frame, 0);
        assert_eq!(y_stride, img.width() as usize);
        assert_eq!(y, y_ref.as_slice(), "Y mismatch frame {i}");
    }

    Ok(())
}

#[test]
fn decode_less_avc_mono12_one_frame() -> Result<()> {
    let img = gen_mono12(16, 16, false)?;
    let (initial, _enc) = LessEncoder::new(&img.view())?;

    let mut stream = Vec::new();
    for nal in initial.into_iter() {
        stream.extend_from_slice(&nal.to_annex_b_data());
    }

    let frames = decode_all_h264_annexb(stream)?;
    assert_eq!(frames.len(), 1);

    let frame = &frames[0];
    assert_eq!(frame.width, img.width());
    assert_eq!(frame.height, img.height());
    assert_eq!(frame.bit_depth, 12);

    let y_ref = img.y_visible_u16().unwrap();
    let (y, y_stride) = plane_u16(frame, 0);
    assert_eq!(y_stride, img.width() as usize);
    assert_eq!(y.len(), y_ref.len());
    assert_eq!(y, y_ref, "mono12 Y mismatch");

    Ok(())
}
