extern crate alloc;

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use oxideav_vp9::{Vp9DecodedFrame, decode_vp9_sequence, split_superframe};

use videoson_core::{
    CodecType, ColorInfo, Packet, PixelFormat, PlaneData, Result, VideoCodecParams, VideoDecoder,
    VideoDecoderOptions, VideoFrame, VideoFramePlanes, VideoOutputFormat, VideoPlane,
    VideosonError, interleave_uv_nv12,
};

struct BufferedPacket {
    data: Vec<u8>,
    pts: Option<i64>,
}

pub struct Vp9Decoder {
    params: VideoCodecParams,
    opts: VideoDecoderOptions,
    packets: Vec<BufferedPacket>,
    queued: VecDeque<VideoFrame>,
}

fn pack_u16_to_u8(src: &[u16]) -> Vec<u8> {
    src.iter().map(|&v| v as u8).collect()
}

fn convert_frame(
    f: Vp9DecodedFrame,
    pts: Option<i64>,
    opts: &VideoDecoderOptions,
) -> Result<VideoFrame> {
    let w = f.width as usize;
    let h = f.height as usize;
    let cw = (w + 1) / 2;
    let ch = (h + 1) / 2;

    if f.bit_depth == 8 {
        let y = pack_u16_to_u8(&f.y);
        let u_data = pack_u16_to_u8(&f.u);
        let v_data = pack_u16_to_u8(&f.v);

        if opts.output_format == VideoOutputFormat::Nv12 {
            let uv = interleave_uv_nv12(&u_data, cw, &v_data, cw, cw, ch);
            Ok(VideoFrame::new_nv12_u8(f.width, f.height, w, cw * 2, y, uv).with_pts(pts))
        } else {
            Ok(
                VideoFrame::new_yuv420_u8(f.width, f.height, w, cw, cw, y, u_data, v_data)
                    .with_pts(pts),
            )
        }
    } else {
        Ok(VideoFrame {
            width: f.width,
            height: f.height,
            planes: VideoFramePlanes::Yuv420,
            pixfmt: PixelFormat::Yuv420,
            bit_depth: f.bit_depth,
            pts,
            plane_data: vec![
                VideoPlane {
                    stride: w,
                    data: PlaneData::U16(f.y),
                },
                VideoPlane {
                    stride: cw,
                    data: PlaneData::U16(f.u),
                },
                VideoPlane {
                    stride: cw,
                    data: PlaneData::U16(f.v),
                },
            ],
            color_info: ColorInfo::default(),
        })
    }
}

impl Vp9Decoder {
    fn drain_packets(&mut self) -> Result<()> {
        if self.packets.is_empty() {
            return Ok(());
        }

        let mut all_data: Vec<Vec<u8>> = Vec::new();
        let mut all_pts: Vec<Option<i64>> = Vec::new();

        for pkt in self.packets.drain(..) {
            let slices = split_superframe(&pkt.data);
            for slice in slices {
                all_data.push(slice.to_vec());
                all_pts.push(pkt.pts);
            }
        }

        let refs: Vec<&[u8]> = all_data.iter().map(|d| d.as_slice()).collect();
        let frames = decode_vp9_sequence(&refs)
            .map_err(|e| VideosonError::Message(alloc::format!("VP9: {e}").into()))?;

        for (i, frame) in frames.into_iter().enumerate() {
            let pts = all_pts.get(i).copied().flatten();
            let vf = convert_frame(frame, pts, &self.opts)?;
            self.queued.push_back(vf);
        }

        Ok(())
    }
}

impl VideoDecoder for Vp9Decoder {
    fn try_new(params: &VideoCodecParams, opts: &VideoDecoderOptions) -> Result<Self> {
        if params.codec != CodecType::VP9 {
            return Err(VideosonError::InvalidData("params.codec is not VP9"));
        }

        if matches!(opts.output_format, VideoOutputFormat::P010) {
            return Err(VideosonError::Unsupported(
                "P010 output is not supported for VP9",
            ));
        }

        Ok(Self {
            params: params.clone(),
            opts: *opts,
            packets: Vec::new(),
            queued: VecDeque::new(),
        })
    }

    fn codec_params(&self) -> &VideoCodecParams {
        &self.params
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        self.packets.push(BufferedPacket {
            data: packet.data.clone(),
            pts: packet.pts,
        });
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>> {
        Ok(self.queued.pop_front())
    }

    fn send_eos(&mut self) -> Result<()> {
        self.drain_packets()
    }

    fn reset(&mut self) {
        self.packets.clear();
        self.queued.clear();
    }

    fn output_format(&self) -> VideoOutputFormat {
        match self.opts.output_format {
            VideoOutputFormat::Nv12 => VideoOutputFormat::Nv12,
            VideoOutputFormat::Native | VideoOutputFormat::Yuv420 => VideoOutputFormat::Yuv420,
            VideoOutputFormat::P010 => VideoOutputFormat::Yuv420,
        }
    }
}
