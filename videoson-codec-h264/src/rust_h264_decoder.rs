// videoson-codec-h264/src/rust_h264_decoder.rs
//
// Full H.264 decoding backend (pure Rust) using the `rust_h264` crate.
// - Works for CABAC + CAVLC, regular I-slices (I4x4/I16x16), P/B frames, etc.
// - Accepts AnnexB or AVCC packets.
// - Can prime the decoder using MP4 `avcC` extradata.
//
// Output: videoson_core::VideoFrame (YUV420, 8-bit, tightly packed, stride == width).

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::string::ToString;

use rust_h264::decoder::Decoder as Inner;
use rust_h264::nal::{parse_annex_b, parse_avcc, parse_avcc_config, NalUnit};

use videoson_core::{
    CodecType, NalFormat, Packet, PlaneData, Result, VideoCodecParams, VideoDecoder,
    VideoDecoderOptions, VideoFrame, VideoFramePlanes, VideoPlane, VideosonError,
};

pub struct RustH264Decoder {
    params: VideoCodecParams,
    _opts: VideoDecoderOptions,

    nal_format: NalFormat,

    // AVCC: how many bytes are used for NAL lengths in samples (1/2/4).
    // If None, fall back to params.nal_format nal_len_size.
    avcc_length_size: Option<usize>,

    dec: Inner,
    out: VecDeque<VideoFrame>,
}

impl RustH264Decoder {
    fn map_err<E: core::fmt::Display>(e: E) -> VideosonError {
        VideosonError::Message(e.to_string().into())
    }

    fn push_frame(&mut self, f: rust_h264::decoder::Frame) {
        // rust_h264 guarantees tightly packed planes:
        // y: width*height, u/v: (width/2)*(height/2)
        let w = f.width as usize;
        let h = f.height as usize;
        let cw = w / 2;
        let ch = h / 2;

        // Sanity checks (avoid panics if upstream changes):
        if f.y.len() != w * h || f.u.len() != cw * ch || f.v.len() != cw * ch {
            // best effort: drop frame
            return;
        }

        self.out.push_back(VideoFrame {
            width: f.width,
            height: f.height,
            planes: VideoFramePlanes::Yuv420,
            pixfmt: videoson_core::PixelFormat::Yuv420,
            bit_depth: 8,
            plane_data: vec![
                VideoPlane {
                    stride: w,
                    data: PlaneData::U8(f.y),
                },
                VideoPlane {
                    stride: cw,
                    data: PlaneData::U8(f.u),
                },
                VideoPlane {
                    stride: cw,
                    data: PlaneData::U8(f.v),
                },
            ],
        });
    }

    fn feed_nal(&mut self, nal: &NalUnit<'_>) -> Result<()> {
        match self.dec.decode_nal(nal) {
            Ok(Some(frame)) => self.push_frame(frame),
            Ok(None) => {}
            Err(e) => return Err(Self::map_err(e)),
        }
        Ok(())
    }

    fn prime_with_avcc_extradata(&mut self) -> Result<()> {
        if self.params.extradata.is_empty() {
            return Ok(());
        }

        // Your MP4 demuxer stores extradata INCLUDING the 8-byte box header (size + "avcC").
        // rust_h264 expects the *payload* only.
        let payload: &[u8] =
            if self.params.extradata.len() >= 8 && &self.params.extradata[4..8] == b"avcC" {
                &self.params.extradata[8..]
            } else {
                self.params.extradata.as_slice()
            };

        let cfg = parse_avcc_config(payload).map_err(Self::map_err)?;
        self.avcc_length_size = Some(cfg.length_size);

        // Feed SPS/PPS once before sample NALs.
        for nal in cfg.sps_nals.iter().chain(cfg.pps_nals.iter()) {
            // No frames expected here; ignore if returned.
            let _ = self.dec.decode_nal(nal).map_err(Self::map_err)?;
        }

        Ok(())
    }

    fn parse_packet_nals<'a>(&self, data: &'a [u8]) -> Vec<NalUnit<'a>> {
        match self.nal_format {
            NalFormat::AnnexB => parse_annex_b(data),
            NalFormat::Avcc { nal_len_size } => {
                let n = self.avcc_length_size.unwrap_or(nal_len_size as usize);
                parse_avcc(data, n)
            }
            _ => parse_annex_b(data),
        }
    }
}

impl VideoDecoder for RustH264Decoder {
    fn try_new(params: &VideoCodecParams, opts: &VideoDecoderOptions) -> Result<Self> {
        if params.codec != CodecType::H264 {
            return Err(VideosonError::InvalidData("params.codec is not H264"));
        }

        let nal_format = params.nal_format.unwrap_or(NalFormat::AnnexB);

        let mut me = Self {
            params: params.clone(),
            _opts: *opts,
            nal_format,
            avcc_length_size: None,
            dec: Inner::new(),
            out: VecDeque::new(),
        };

        // If this is MP4/AVCC, prime with avcC SPS/PPS (if present).
        // If stream is AnnexB and includes SPS/PPS inline, this is a no-op.
        if matches!(me.nal_format, NalFormat::Avcc { .. }) {
            me.prime_with_avcc_extradata()?;
        }

        Ok(me)
    }

    fn codec_params(&self) -> &VideoCodecParams {
        &self.params
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        let nals = self.parse_packet_nals(&packet.data);
        for nal in &nals {
            self.feed_nal(nal)?;
        }
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>> {
        Ok(self.out.pop_front())
    }

    fn send_eos(&mut self) -> Result<()> {
        if let Some(frame) = self.dec.flush() {
            self.push_frame(frame);
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.dec = Inner::new();
        self.out.clear();
        self.avcc_length_size = None;

        if matches!(self.nal_format, NalFormat::Avcc { .. }) {
            let _ = self.prime_with_avcc_extradata();
        }
    }
}
