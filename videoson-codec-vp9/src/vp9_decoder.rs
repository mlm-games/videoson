extern crate alloc;

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use oxideav_core::ExecutionContext;
use oxideav_vp9::{Vp9DecodedFrame, Vp9SequenceDecoder, split_superframe};

use videoson_core::{
    CodecType, ColorInfo, Packet, PixelFormat, PlaneData, Result, VideoCodecParams, VideoDecoder,
    VideoDecoderOptions, VideoFrame, VideoFramePlanes, VideoOutputFormat, VideoPlane,
    VideosonError, interleave_uv_nv12, require_plane_len,
};

pub struct Vp9Decoder {
    params: VideoCodecParams,
    opts: VideoDecoderOptions,
    decoder: Vp9SequenceDecoder,
    queued: VecDeque<VideoFrame>,
    exec: ExecutionContext,
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
    let cw = if f.subsampling_x { (w + 1) / 2 } else { w };
    let ch = if f.subsampling_y { (h + 1) / 2 } else { h };

    if !f.subsampling_x || !f.subsampling_y {
        let has_chroma = !f.u.is_empty() || !f.v.is_empty();
        if has_chroma {
            return Err(VideosonError::Unsupported(
                "VP9: only 4:2:0 chroma is supported",
            ));
        }
    }

    require_plane_len(f.y.len(), w, w, h, "VP9: Y plane too short")?;
    if !f.u.is_empty() {
        require_plane_len(f.u.len(), cw, cw, ch, "VP9: U plane too short")?;
    }
    if !f.v.is_empty() {
        require_plane_len(f.v.len(), cw, cw, ch, "VP9: V plane too short")?;
    }

    if f.u.is_empty() && f.v.is_empty() {
        if f.bit_depth == 8 {
            let y = pack_u16_to_u8(&f.y);
            return Ok(VideoFrame {
                width: f.width,
                height: f.height,
                planes: VideoFramePlanes::Mono,
                pixfmt: PixelFormat::Gray,
                bit_depth: 8,
                pts,
                plane_data: vec![VideoPlane {
                    stride: w,
                    data: PlaneData::U8(y),
                }],
                color_info: ColorInfo::default(),
                poc: None,
            });
        } else {
            return Ok(VideoFrame {
                width: f.width,
                height: f.height,
                planes: VideoFramePlanes::Mono,
                pixfmt: PixelFormat::Gray,
                bit_depth: f.bit_depth,
                pts,
                plane_data: vec![VideoPlane {
                    stride: w,
                    data: PlaneData::U16(f.y),
                }],
                color_info: ColorInfo::default(),
                poc: None,
            });
        }
    }

    if f.bit_depth == 8 {
        let y = pack_u16_to_u8(&f.y);
        let u_data = pack_u16_to_u8(&f.u);
        let v_data = pack_u16_to_u8(&f.v);

        if opts.output_format == VideoOutputFormat::Nv12 {
            let uv = interleave_uv_nv12(&u_data, cw, &v_data, cw, cw, ch)?;
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
            poc: None,
        })
    }
}

impl Vp9Decoder {
    fn exec_from_opts(opts: &VideoDecoderOptions) -> ExecutionContext {
        match opts.threads {
            Some(n) => ExecutionContext::with_threads(n),
            None => ExecutionContext::serial(),
        }
    }

    /// Override the thread budget after construction. Mirrors the
    /// `oxideav_core::ExecutionContext` contract: the budget is advisory
    /// and preserved across `reset()`.
    pub fn set_execution_context(&mut self, exec: &ExecutionContext) {
        self.exec = exec.clone();
        self.decoder.set_execution_context(exec);
    }

    /// Convenience: set thread count directly.
    pub fn set_threads(&mut self, threads: usize) {
        let exec = ExecutionContext::with_threads(threads);
        self.set_execution_context(&exec);
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

        let exec = Self::exec_from_opts(opts);
        let mut decoder = Vp9SequenceDecoder::new();
        decoder.set_execution_context(&exec);

        Ok(Self {
            params: params.clone(),
            opts: *opts,
            decoder,
            queued: VecDeque::new(),
            exec,
        })
    }

    fn codec_params(&self) -> &VideoCodecParams {
        &self.params
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        let slices = split_superframe(&packet.data);
        for slice in slices {
            let maybe_frame = self
                .decoder
                .push_frame(slice)
                .map_err(|e| VideosonError::Message(alloc::format!("VP9: {e}").into()))?;
            if let Some(frame) = maybe_frame {
                let vf = convert_frame(frame, packet.pts, &self.opts)?;
                self.queued.push_back(vf);
            }
        }
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>> {
        Ok(self.queued.pop_front())
    }

    fn send_eos(&mut self) -> Result<()> {
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.decoder = Vp9SequenceDecoder::new();
        self.decoder.set_execution_context(&self.exec);
        self.queued.clear();
        Ok(())
    }

    fn requested_output_format(&self) -> VideoOutputFormat {
        match self.opts.output_format {
            VideoOutputFormat::Nv12 => VideoOutputFormat::Nv12,
            VideoOutputFormat::Native | VideoOutputFormat::Yuv420 => VideoOutputFormat::Yuv420,
            // P010 not supported for VP9; returns Yuv420 or Yuv420-U16 for high-bitdepth
            VideoOutputFormat::P010 => VideoOutputFormat::Yuv420,
        }
    }
}
