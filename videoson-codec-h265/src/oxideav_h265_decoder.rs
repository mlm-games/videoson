extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::collections::{BinaryHeap, VecDeque};
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Reverse;

use oxideav_core::ExecutionContext;
use oxideav_h265::hvcc::{extradata_is_hvcc, parse_hvcc, split_length_prefixed};
use oxideav_h265::nal::NalUnit;
use oxideav_h265::picture::{Picture, Plane};
use oxideav_h265::sequence::{DecodedFrame, SequenceDecoder};
use oxideav_h265::sps::SeqParameterSet;

use videoson_core::{
    CICP_UNSPECIFIED, CodecType, ColorInfo, Packet, PixelFormat, PlaneData, Result,
    VideoCodecParams, VideoDecoder, VideoDecoderOptions, VideoFrame, VideoFramePlanes,
    VideoOutputFormat, VideoPlane, VideosonError, interleave_uv_nv12, require_plane_len,
};

/// Default output-reorder depth before `sps_max_num_reorder_pics` is
/// known. Mirrors oxideav's registry decoder.
const DEFAULT_REORDER: usize = 8;

/// `sps_seq_parameter_set_id` NAL type, Table 7-1.
const NAL_UNIT_TYPE_SPS: u8 = 33;

/// Pull the CICP triple out of the first SPS in `units`.
///
/// §E.3.1: an absent `vui_parameters`, an absent
/// `video_signal_type`, or `colour_description_present_flag == 0`
/// all leave the fields unspecified, reported here as `None` so
/// callers can apply their own fallback instead of reading zeros as
/// a real signal.
pub fn cicp_from_nal_units(units: &[NalUnit]) -> Option<ColorInfo> {
    let sps = units
        .iter()
        .find(|u| u.header.nal_unit_type == NAL_UNIT_TYPE_SPS)
        .and_then(|u| SeqParameterSet::parse(&u.rbsp).ok())?;
    let signal = sps.vui_parameters?.video_signal_type?;
    let d = signal.colour_description;
    Some(ColorInfo {
        primaries: d.as_ref().map_or(CICP_UNSPECIFIED, |d| d.colour_primaries),
        transfer: d
            .as_ref()
            .map_or(CICP_UNSPECIFIED, |d| d.transfer_characteristics),
        matrix: d.as_ref().map_or(CICP_UNSPECIFIED, |d| d.matrix_coeffs),
        full_range: signal.video_full_range_flag,
        signalled: true,
    })
}

/// Strip an 8-byte `hvcC` box header (`size + 'hvcC'`) when present,
/// returning the raw HEVCDecoderConfigurationRecord.
fn strip_hvcc_box(extradata: &[u8]) -> &[u8] {
    if extradata.len() >= 8 && &extradata[4..8] == b"hvcC" {
        &extradata[8..]
    } else {
        extradata
    }
}

fn map_err(e: impl core::fmt::Display) -> VideosonError {
    VideosonError::Message(alloc::format!("H.265 oxideav: {e}").into())
}

fn pack_plane_u8(buf: &[u16], w: usize, h: usize, max: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h);
    for &s in buf.iter().take(w * h) {
        out.push(s.min(max) as u8);
    }
    out
}

fn pack_plane_u16(buf: &[u16], w: usize, h: usize, max: u16) -> Vec<u16> {
    let mut out = Vec::with_capacity(w * h);
    for &s in buf.iter().take(w * h) {
        out.push(s.min(max));
    }
    out
}

/// Convert one reconstructed `Picture` into a videoson `VideoFrame`
/// with exact dimensions and bit depth (8-bit U8 / high-bit-depth
/// U16, monochrome or 4:2:0 / 4:2:2 / 4:4:4 planar).
fn convert_picture(
    pic: &Picture,
    pts: Option<i64>,
    opts: &VideoDecoderOptions,
    color: ColorInfo,
) -> Result<VideoFrame> {
    let (w, h) = pic.plane_dims(Plane::Luma);
    let bit_depth = pic.bit_depth(Plane::Luma);
    let mono = pic.chroma_array_type() == 0;

    if mono {
        if bit_depth == 8 {
            let y = pack_plane_u8(pic.plane(Plane::Luma), w, h, u8::MAX as u16);
            require_plane_len(y.len(), w, w, h, "H.265: Y plane too short")?;
            return Ok(VideoFrame {
                width: w as u32,
                height: h as u32,
                planes: VideoFramePlanes::Mono,
                pixfmt: PixelFormat::Gray,
                bit_depth,
                pts,
                plane_data: vec![VideoPlane {
                    stride: w,
                    data: PlaneData::U8(y),
                }],
                color_info: color,
                poc: None,
            });
        }
        let max = ((1u32 << bit_depth) - 1) as u16;
        let y = pack_plane_u16(pic.plane(Plane::Luma), w, h, max);
        return Ok(VideoFrame {
            width: w as u32,
            height: h as u32,
            planes: VideoFramePlanes::Mono,
            pixfmt: PixelFormat::Gray,
            bit_depth,
            pts,
            plane_data: vec![VideoPlane {
                stride: w,
                data: PlaneData::U16(y),
            }],
            color_info: color,
            poc: None,
        });
    }

    let (planes, pixfmt) = match pic.chroma_array_type() {
        1 => (VideoFramePlanes::Yuv420, PixelFormat::Yuv420),
        2 => (VideoFramePlanes::Yuv422, PixelFormat::Yuv422),
        3 => (VideoFramePlanes::Yuv444, PixelFormat::Yuv444),
        _ => {
            return Err(VideosonError::Unsupported(
                "H.265: unsupported chroma_format_idc",
            ));
        }
    };
    let (cw, ch) = pic.plane_dims(Plane::Cb);

    if bit_depth == 8 {
        let max = u8::MAX as u16;
        let y = pack_plane_u8(pic.plane(Plane::Luma), w, h, max);
        let u = pack_plane_u8(pic.plane(Plane::Cb), cw, ch, max);
        let v = pack_plane_u8(pic.plane(Plane::Cr), cw, ch, max);
        require_plane_len(y.len(), w, w, h, "H.265: Y plane too short")?;
        require_plane_len(u.len(), cw, cw, ch, "H.265: U plane too short")?;
        require_plane_len(v.len(), cw, cw, ch, "H.265: V plane too short")?;
        if matches!(opts.output_format, VideoOutputFormat::Nv12) && pixfmt == PixelFormat::Yuv420 {
            let uv = interleave_uv_nv12(&u, cw, &v, cw, cw, ch)?;
            return Ok(
                VideoFrame::new_nv12_u8(w as u32, h as u32, w, cw * 2, y, uv)
                    .with_pts(pts)
                    .with_color_info(color),
            );
        }
        let frame = match pixfmt {
            PixelFormat::Yuv420 => {
                VideoFrame::new_yuv420_u8(w as u32, h as u32, w, cw, cw, y, u, v)
            }
            PixelFormat::Yuv422 => {
                VideoFrame::new_yuv422_u8(w as u32, h as u32, w, cw, cw, y, u, v)
            }
            PixelFormat::Yuv444 => {
                VideoFrame::new_yuv444_u8(w as u32, h as u32, w, cw, cw, y, u, v)
            }
            _ => {
                return Err(VideosonError::Unsupported(
                    "H.265: unsupported chroma_format_idc",
                ));
            }
        };
        Ok(frame.with_pts(pts).with_color_info(color))
    } else {
        let max = ((1u32 << bit_depth) - 1) as u16;
        let y = pack_plane_u16(pic.plane(Plane::Luma), w, h, max);
        let u = pack_plane_u16(pic.plane(Plane::Cb), cw, ch, max);
        let v = pack_plane_u16(pic.plane(Plane::Cr), cw, ch, max);
        require_plane_len(y.len(), w, w, h, "H.265: Y plane too short")?;
        require_plane_len(u.len(), cw, cw, ch, "H.265: U plane too short")?;
        require_plane_len(v.len(), cw, cw, ch, "H.265: V plane too short")?;
        Ok(VideoFrame {
            width: w as u32,
            height: h as u32,
            planes,
            pixfmt,
            bit_depth,
            pts,
            plane_data: vec![
                VideoPlane {
                    stride: w,
                    data: PlaneData::U16(y),
                },
                VideoPlane {
                    stride: cw,
                    data: PlaneData::U16(u),
                },
                VideoPlane {
                    stride: cw,
                    data: PlaneData::U16(v),
                },
            ],
            color_info: color,
            poc: None,
        })
    }
}

pub struct OxideH265Decoder {
    params: VideoCodecParams,
    opts: VideoDecoderOptions,
    seq: SequenceDecoder,
    exec: ExecutionContext,
    nal_length_size: Option<usize>,
    color: ColorInfo,
    reorder: Vec<DecodedFrame>,
    ready: VecDeque<VideoFrame>,
    pts_queue: BinaryHeap<Reverse<i64>>,
    flushed: bool,
}

impl OxideH265Decoder {
    fn prime_extradata(
        seq: &mut SequenceDecoder,
        extradata: &[u8],
    ) -> Result<(Option<usize>, Option<ColorInfo>)> {
        let payload = strip_hvcc_box(extradata);
        if payload.is_empty() {
            return Ok((None, None));
        }
        if extradata_is_hvcc(payload) {
            let rec = parse_hvcc(payload).map_err(map_err)?;
            let color = cicp_from_nal_units(&rec.nal_units);
            for unit in rec.nal_units {
                seq.push_nal_unit(unit).map_err(map_err)?;
            }
            Ok((Some(rec.length_size), color))
        } else {
            seq.push_annexb(payload).map_err(map_err)?;
            let color = oxideav_h265::nal::collect_nal_units(payload)
                .map_or(None, |units| cicp_from_nal_units(&units));
            Ok((None, color))
        }
    }

    fn drain(&mut self, flush: bool) -> Result<()> {
        self.reorder.extend(self.seq.take_decoded());
        self.reorder.sort_by_key(|f| (f.cvs_index, f.poc));
        let depth = if flush {
            0
        } else {
            self.seq
                .max_num_reorder_pics()
                .map_or(DEFAULT_REORDER, |n| n as usize)
        };
        while self.reorder.len() > depth {
            let f = self.reorder.remove(0);
            if !f.output {
                continue;
            }
            let pts = self.pts_queue.pop().map(|r| r.0);
            let poc = f.poc;
            let pic = f.output_picture();
            let mut vf = convert_picture(&pic, pts, &self.opts, self.color)?;
            vf.poc = Some(poc);
            self.ready.push_back(vf);
        }
        Ok(())
    }

    fn build(
        params: &VideoCodecParams,
        exec: &ExecutionContext,
    ) -> Result<(SequenceDecoder, Option<usize>, Option<ColorInfo>)> {
        let mut seq = SequenceDecoder::new();
        seq.set_threads(exec.threads);
        let (nal_length_size, color) = Self::prime_extradata(&mut seq, &params.extradata)?;
        Ok((seq, nal_length_size, color))
    }

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
        self.seq.set_threads(exec.threads);
    }

    /// Convenience: set thread count directly.
    pub fn set_threads(&mut self, threads: usize) {
        let exec = ExecutionContext::with_threads(threads);
        self.set_execution_context(&exec);
    }
}

impl VideoDecoder for OxideH265Decoder {
    fn try_new(params: &VideoCodecParams, opts: &VideoDecoderOptions) -> Result<Self>
    where
        Self: Sized,
    {
        if params.codec != CodecType::H265 {
            return Err(VideosonError::InvalidData("params.codec is not H265"));
        }
        if matches!(opts.output_format, VideoOutputFormat::P010) {
            return Err(VideosonError::Unsupported(
                "P010 output is not supported for H.265",
            ));
        }
        let exec = Self::exec_from_opts(opts);
        let (seq, nal_length_size, color) = Self::build(params, &exec)?;
        Ok(Self {
            params: params.clone(),
            opts: *opts,
            seq,
            exec,
            nal_length_size,
            color: color.unwrap_or_default(),
            reorder: Vec::new(),
            ready: VecDeque::new(),
            pts_queue: BinaryHeap::new(),
            flushed: false,
        })
    }

    fn codec_params(&self) -> &VideoCodecParams {
        &self.params
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if packet.data.is_empty() {
            return self.send_eos();
        }
        if let Some(pts) = packet.pts {
            self.pts_queue.push(Reverse(pts));
        }
        if let Some(length_size) = self.nal_length_size {
            let units = split_length_prefixed(&packet.data, length_size).map_err(map_err)?;
            if let Some(color) = cicp_from_nal_units(&units) {
                self.color = color;
            }
            for unit in units {
                self.seq.push_nal_unit(unit).map_err(map_err)?;
            }
        } else {
            if let Some(color) = oxideav_h265::nal::collect_nal_units(&packet.data)
                .ok()
                .as_deref()
                .and_then(cicp_from_nal_units)
            {
                self.color = color;
            }
            self.seq.push_annexb(&packet.data).map_err(map_err)?;
        }
        self.drain(false)
    }

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>> {
        Ok(self.ready.pop_front())
    }

    fn send_eos(&mut self) -> Result<()> {
        self.seq.flush().map_err(map_err)?;
        self.flushed = true;
        self.drain(true)
    }

    fn reset(&mut self) -> Result<()> {
        let (seq, nal_length_size, color) = Self::build(&self.params, &self.exec)?;
        self.seq = seq;
        self.nal_length_size = nal_length_size;
        self.color = color.unwrap_or_default();
        self.reorder.clear();
        self.ready.clear();
        self.pts_queue.clear();
        self.flushed = false;
        Ok(())
    }

    fn requested_output_format(&self) -> VideoOutputFormat {
        match self.opts.output_format {
            VideoOutputFormat::Nv12 => VideoOutputFormat::Nv12,
            VideoOutputFormat::Native | VideoOutputFormat::Yuv420 => VideoOutputFormat::Yuv420,
            VideoOutputFormat::P010 => VideoOutputFormat::Yuv420,
        }
    }
}
