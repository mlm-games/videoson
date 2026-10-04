extern crate alloc;

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use oxideav_core::ExecutionContext;
use oxideav_vp9::{
    ColorConfig, ColorSpace, FrameType, Vp9DecodedFrame, Vp9SequenceDecoder,
    parse_uncompressed_header, split_superframe,
};

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
    /// Colour carried from the last frame whose header actually signalled it.
    ///
    /// §6.2 only lets an intra frame establish `color_config( )`; inter frames
    /// inherit it. An inter header cannot be parsed standalone — it needs the
    /// reference state this decoder keeps internally — so the value is
    /// latched here instead of re-read per frame.
    color: ColorInfo,
}

/// Map a VP9 §7.2.2 `color_config( )` onto H.273 code points.
///
/// `None` when the config carries no signal. `CS_UNKNOWN` states outright that
/// colour comes out-of-band and `CS_RESERVED` is undefined; inventing a
/// concrete code for either would be worse than reporting nothing.
fn cicp_from_color_config(cc: ColorConfig) -> Option<ColorInfo> {
    let (primaries, transfer, matrix) = match cc.color_space {
        ColorSpace::Unknown | ColorSpace::Reserved => return None,
        // VP9 defines both of these as BT.601-family. H.273 code 6 (SMPTE
        // 170M) carries the coefficients that match.
        ColorSpace::Bt601 | ColorSpace::Smpte170 => (6, 6, 6),
        ColorSpace::Bt709 => (1, 1, 1),
        ColorSpace::Smpte240 => (7, 7, 7),
        ColorSpace::Bt2020 => (9, 9, 9),
        // CS_RGB is 4:4:4, so the luma coefficients are the identity matrix —
        // H.273 code 0. CICP has no sRGB code point; BT.709 primaries and
        // transfer are the conventional stand-in.
        ColorSpace::Rgb => (1, 1, 0),
    };
    Some(ColorInfo {
        primaries,
        transfer,
        matrix,
        full_range: cc.color_range_full,
        signalled: true,
    })
}

fn pack_u16_to_u8(src: &[u16]) -> Vec<u8> {
    src.iter().map(|&v| v as u8).collect()
}

fn convert_frame(
    f: Vp9DecodedFrame,
    pts: Option<i64>,
    opts: &VideoDecoderOptions,
    color: ColorInfo,
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
                color_info: color,
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
                color_info: color,
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
            Ok(VideoFrame::new_nv12_u8(f.width, f.height, w, cw * 2, y, uv)
                .with_pts(pts)
                .with_color_info(color))
        } else {
            Ok(
                VideoFrame::new_yuv420_u8(f.width, f.height, w, cw, cw, y, u_data, v_data)
                    .with_pts(pts)
                    .with_color_info(color),
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
            color_info: color,
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

    /// Latch the colour an intra frame's uncompressed header carries.
    ///
    /// A parse failure is not an error: inter headers need reference state and
    /// always fail here, and colour is metadata that must never fail a decode.
    /// The `show_existing_frame` sentinel early-returns a header full of
    /// placeholder fields, and the frame it re-displays inherits whatever was
    /// latched when that frame was originally decoded, so both are skipped.
    fn latch_color(&mut self, slice: &[u8]) {
        let Ok(hdr) = parse_uncompressed_header(slice) else {
            return;
        };
        if hdr.show_existing_frame {
            return;
        }
        // A key frame always reads colour from the bitstream. A non-key intra
        // frame does only when Profile > 0; at Profile 0 §6.2 installs BT.601
        // as a default rather than reading it, which is not a signal.
        if !(hdr.frame_type == FrameType::KeyFrame || hdr.profile > 0) {
            return;
        }
        if let Some(cicp) = cicp_from_color_config(hdr.color_config) {
            self.color = cicp;
        }
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
            color: ColorInfo::default(),
        })
    }

    fn codec_params(&self) -> &VideoCodecParams {
        &self.params
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        let slices = split_superframe(&packet.data);
        for slice in slices {
            self.latch_color(slice);
            let maybe_frame = self
                .decoder
                .push_frame(slice)
                .map_err(|e| VideosonError::Message(alloc::format!("VP9: {e}").into()))?;
            if let Some(frame) = maybe_frame {
                let vf = convert_frame(frame, packet.pts, &self.opts, self.color)?;
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
        self.color = ColorInfo::default();
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
