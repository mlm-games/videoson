extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "std")]
macro_rules! dbg_log {
    ($($arg:tt)*) => { std::eprintln!($($arg)*); };
}
#[cfg(not(feature = "std"))]
macro_rules! dbg_log {
    ($($arg:tt)*) => {};
}

use alloc::borrow::Cow;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

use rust_h265::{Decoder, NalUnit, NalUnitType, parse_annex_b};

use videoson_common::parse_hvcc_extradata;

use videoson_core::{
    CodecType, NalFormat, Packet, Result, VideoCodecParams, VideoDecoder, VideoDecoderOptions,
    VideoFrame, VideoOutputFormat, VideosonError, interleave_uv_nv12, require_plane_len,
};

struct OrderedFrame {
    gop: u32,
    poc: i32,
    frame: VideoFrame,
}

fn parse_nal(nal_data: &[u8]) -> Option<NalUnit<'_>> {
    if nal_data.len() < 2 {
        return None;
    }
    let b0 = nal_data[0];
    let b1 = nal_data[1];
    if b0 & 0x80 != 0 {
        return None;
    }
    let nal_unit_type = NalUnitType::from((b0 >> 1) & 0x3F);
    let nuh_layer_id = ((b0 & 0x01) << 5) | (b1 >> 3);
    let temporal_id_plus1 = b1 & 0x07;
    if temporal_id_plus1 == 0 {
        return None;
    }
    let temporal_id = temporal_id_plus1 - 1;
    let (rbsp, epb_positions) = remove_epb(&nal_data[2..]);
    Some(NalUnit {
        nal_unit_type,
        nuh_layer_id,
        temporal_id,
        rbsp,
        epb_positions,
    })
}

fn remove_epb(data: &[u8]) -> (Cow<'_, [u8]>, Vec<u32>) {
    let has_epb = data.windows(3).any(|w| w[0] == 0 && w[1] == 0 && w[2] == 3);
    if !has_epb {
        return (Cow::Borrowed(data), Vec::new());
    }
    let mut rbsp = Vec::with_capacity(data.len());
    let mut epb_positions = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if i + 2 < data.len() && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 3 {
            rbsp.push(0);
            rbsp.push(0);
            epb_positions.push((i + 2) as u32);
            i += 3;
        } else {
            rbsp.push(data[i]);
            i += 1;
        }
    }
    (Cow::Owned(rbsp), epb_positions)
}

fn parse_hvcc(data: &[u8], length_size: u8) -> Vec<NalUnit<'_>> {
    if length_size == 0 || length_size > 4 {
        return Vec::new();
    }
    let ls = length_size as usize;
    let mut nals = Vec::new();
    let mut i = 0;
    while i + ls <= data.len() {
        let mut nal_len: usize = 0;
        for j in 0..ls {
            nal_len = (nal_len << 8) | data[i + j] as usize;
        }
        i += ls;
        if nal_len == 0 || i + nal_len > data.len() {
            break;
        }
        let nal_data = &data[i..i + nal_len];
        if let Some(nal) = parse_nal(nal_data) {
            nals.push(nal);
        }
        i += nal_len;
    }
    nals
}

pub struct RustH265Decoder {
    params: VideoCodecParams,
    opts: VideoDecoderOptions,
    nal_format: NalFormat,
    dec: Decoder,
    queued: VecDeque<VideoFrame>,
    pending: Vec<OrderedFrame>,
    gop_count: u32,
    in_irap_picture: bool,
    hvcc_length_size: Option<u8>,
    pending_pts: Option<i64>,
    /// Frame duration in microseconds, used to recompute PTS from POC.
    /// 0 means "not set" (fall back to container PTS).
    frame_duration_us: u64,
}

impl RustH265Decoder {
    fn map_err(e: rust_h265::DecodeError) -> VideosonError {
        VideosonError::Message(alloc::format!("H.265: {e:?}").into())
    }

    fn wants_nv12(&self) -> bool {
        matches!(self.opts.output_format, VideoOutputFormat::Nv12)
    }

    fn flush_pending_for_gop(&mut self, gop: u32) {
        let mut gop_frames = Vec::new();
        let mut remaining = Vec::new();
        for f in self.pending.drain(..) {
            if f.gop == gop {
                gop_frames.push(f);
            } else {
                remaining.push(f);
            }
        }
        self.pending = remaining;
        gop_frames.sort_by(|a, b| a.poc.cmp(&b.poc));
        for ordered in gop_frames {
            self.queued.push_back(ordered.frame);
        }
    }

    fn flush_ready_frames(&mut self) {
        if self.pending.len() <= 1 {
            return;
        }
        self.pending.sort_by(|a, b| a.poc.cmp(&b.poc));
        let max_poc = self.pending.last().unwrap().poc;
        let mut ready = Vec::new();
        let mut remaining = Vec::new();
        for f in self.pending.drain(..) {
            if f.poc < max_poc {
                ready.push(f);
            } else {
                remaining.push(f);
            }
        }
        self.pending = remaining;
        for ordered in ready {
            self.queued.push_back(ordered.frame);
        }
    }

    fn drain_all_pending(&mut self) {
        self.pending.sort_by(|a, b| match a.gop.cmp(&b.gop) {
            core::cmp::Ordering::Equal => a.poc.cmp(&b.poc),
            other => other,
        });
        for ordered in self.pending.drain(..) {
            self.queued.push_back(ordered.frame);
        }
    }

    fn push_frame(&mut self, f: rust_h265::decoder::Frame, poc: i32, pts: Option<i64>) -> Result<()> {
        // Correct PTS from POC when frame_duration_us is available.
        // This fixes mis-muxed files where the container PTS assumes B-frame
        // reordering but the bitstream has no B-frames (POC is sequential).
        let pts = if self.frame_duration_us > 0 {
            Some(poc as i64 * self.frame_duration_us as i64 * 1000) // Miniter uses nanosecs for precision...
        } else {
            pts
        };
        let w = f.width as usize;
        let h = f.height as usize;
        let cw = (w + 1) / 2;
        let ch = (h + 1) / 2;
        let bd = f.bit_depth;

        let mut frame: VideoFrame = match bd {
            8 => self.make_frame_u8(f, w, h, cw, ch, pts)?,
            10 | 12 => self.make_frame_u16(f, w, h, cw, ch, bd, pts)?,
            _ => {
                return Err(VideosonError::Unsupported(
                    "H.265: unsupported bit depth",
                ))
            }
        };
        frame.poc = Some(poc);

        let gop = self.gop_count;
        self.pending.push(OrderedFrame { gop, poc, frame });
        Ok(())
    }

    fn check_chroma_420(&self, u_len: usize, v_len: usize, cw: usize, ch: usize) -> Result<()> {
        if (u_len > 0 && (u_len / cw.max(1)) > ch)
            || (v_len > 0 && (v_len / cw.max(1)) > ch)
        {
            return Err(VideosonError::Unsupported(
                "H.265: only 4:2:0 chroma is supported",
            ));
        }
        Ok(())
    }

    fn make_frame_u8(
        &self,
        f: rust_h265::decoder::Frame,
        w: usize,
        _h: usize,
        cw: usize,
        ch: usize,
        pts: Option<i64>,
    ) -> Result<VideoFrame> {
        let y = f.y.as_u8().ok_or(VideosonError::InvalidData(
            "H.265: expected U8 pixel data for bit_depth=8",
        ))?;
        let u = f.u.as_u8().ok_or(VideosonError::InvalidData(
            "H.265: expected U8 chroma for bit_depth=8",
        ))?;
        let v = f.v.as_u8().ok_or(VideosonError::InvalidData(
            "H.265: expected U8 chroma for bit_depth=8",
        ))?;
        require_plane_len(y.len(), w, w, f.height as usize, "H.265: Y plane too short")?;
        require_plane_len(u.len(), cw, cw, ch, "H.265: U plane too short")?;
        require_plane_len(v.len(), cw, cw, ch, "H.265: V plane too short")?;
        self.check_chroma_420(u.len(), v.len(), cw, ch)?;

        if self.wants_nv12() {
            let uv = interleave_uv_nv12(&u, cw, &v, cw, cw, ch)?;
            Ok(VideoFrame::new_nv12_u8(f.width, f.height, w, cw * 2, y.to_vec(), uv)
                .with_pts(pts))
        } else {
            Ok(VideoFrame::new_yuv420_u8(
                f.width, f.height, w, cw, cw, y.to_vec(), u.to_vec(), v.to_vec(),
            )
            .with_pts(pts))
        }
    }

    fn make_frame_u16(
        &self,
        f: rust_h265::decoder::Frame,
        w: usize,
        _h: usize,
        cw: usize,
        ch: usize,
        bd: u8,
        pts: Option<i64>,
    ) -> Result<VideoFrame> {
        let y = f.y.as_u16().ok_or(VideosonError::InvalidData(
            "H.265: expected U16 pixel data for high bit depth",
        ))?;
        let u = f.u.as_u16().ok_or(VideosonError::InvalidData(
            "H.265: expected U16 chroma for high bit depth",
        ))?;
        let v = f.v.as_u16().ok_or(VideosonError::InvalidData(
            "H.265: expected U16 chroma for high bit depth",
        ))?;
        require_plane_len(y.len(), w, w, f.height as usize, "H.265: Y plane too short")?;
        require_plane_len(u.len(), cw, cw, ch, "H.265: U plane too short")?;
        require_plane_len(v.len(), cw, cw, ch, "H.265: V plane too short")?;
        self.check_chroma_420(u.len(), v.len(), cw, ch)?;

        Ok(VideoFrame::new_yuv420_u16(
            f.width, f.height, w, cw, cw, y.to_vec(), u.to_vec(), v.to_vec(), bd,
        )
        .with_pts(pts))
    }

    fn feed_nal(&mut self, nal: &NalUnit<'_>, pts: Option<i64>) -> Result<()> {
        let is_irap = nal.nal_unit_type.is_irap();

        self.dec.set_pending_pts(pts);

        let dec_result = self.dec.decode_nal(nal);
        match dec_result {
            Ok(Some(frame)) => {
                let poc = frame.pic_order_cnt;
                let frame_pts = frame.pts;
                self.push_frame(frame, poc, frame_pts)?;
            }
            Ok(None) => {}
            Err(e) => return Err(Self::map_err(e)),
        }

        // Continuous flush: emit all pending frames except the one with the
        // highest POC - ensures frames flow with minimal latency while
        // preserving POC order.
        self.flush_ready_frames();

        // IRAP/GOP handling fires once per IRAP access unit, not per NAL.
        // Flush the old GOP before starting the new one.
        if is_irap && !self.in_irap_picture {
            if self.pending.iter().any(|f| f.gop == self.gop_count) {
                self.flush_pending_for_gop(self.gop_count);
            }
            self.gop_count += 1;
            self.in_irap_picture = true;
        }

        // Clear the IRAP guard on the first non-IRAP VCL NAL.
        if nal.nal_unit_type.is_vcl() && !is_irap {
            self.in_irap_picture = false;
        }

        // Track PTS for the picture currently being decoded.
        // Used by send_eos to attach the correct PTS to the flushed frame.
        if nal.nal_unit_type.is_vcl() {
            self.pending_pts = pts;
        }

        Ok(())
    }

    fn parse_packet_nals<'a>(&self, data: &'a [u8]) -> Vec<NalUnit<'a>> {
        match self.nal_format {
            NalFormat::AnnexB => parse_annex_b(data),
            NalFormat::Hvcc { nal_len_size } => {
                let n = self.hvcc_length_size.unwrap_or(nal_len_size);
                parse_hvcc(data, n)
            }
            _ => parse_annex_b(data),
        }
    }

    fn prime_with_hvcc_extradata(&mut self) -> Result<()> {
        let extradata = self.params.extradata.clone();
        if extradata.is_empty() {
            return Ok(());
        }

        let payload: Vec<u8> = if extradata.len() >= 8 && &extradata[4..8] == b"hvcC" {
            extradata[8..].to_vec()
        } else {
            extradata
        };

        let cfg = parse_hvcc_extradata(&payload)
            .map_err(|e| VideosonError::Message(alloc::format!("hvcC: {e:?}").into()))?;
        self.hvcc_length_size = Some(cfg.nal_len_size);

        let nal_list: Vec<Vec<u8>> = videoson_common::hvcc_nal_bytes(&payload)
            .collect::<core::result::Result<Vec<_>, _>>()
            .map_err(|e| VideosonError::Message(alloc::format!("hvcC NAL parse: {e:?}").into()))?
            .iter()
            .map(|b| b.to_vec())
            .collect();

        for nal_bytes in &nal_list {
            if let Some(nal) = parse_nal(nal_bytes) {
                self.feed_nal(&nal, None)?;
            }
        }
        Ok(())
    }
}

impl VideoDecoder for RustH265Decoder {
    fn try_new(params: &VideoCodecParams, opts: &VideoDecoderOptions) -> Result<Self> {
        if params.codec != CodecType::H265 {
            return Err(VideosonError::InvalidData("params.codec is not H265"));
        }

        if matches!(opts.output_format, VideoOutputFormat::P010) {
            return Err(VideosonError::Unsupported(
                "P010 output is not supported for H.265 (use Native/Yuv420/Nv12)",
            ));
        }

        let nal_format = params.nal_format.unwrap_or(NalFormat::AnnexB);

        let mut me = Self {
            params: params.clone(),
            opts: *opts,
            nal_format,
            dec: Decoder::new(),
            queued: VecDeque::new(),
            pending: Vec::new(),
            gop_count: 0,
            in_irap_picture: false,
            hvcc_length_size: None,
            pending_pts: None,
            frame_duration_us: 0,
        };

        if matches!(me.nal_format, NalFormat::Hvcc { .. }) {
            me.prime_with_hvcc_extradata()?;
        }

        Ok(me)
    }

    fn codec_params(&self) -> &VideoCodecParams {
        &self.params
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        let pts = packet.pts;
        let nals = self.parse_packet_nals(&packet.data);
        for nal in &nals {
            self.feed_nal(nal, pts)?;
        }
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>> {
        Ok(self.queued.pop_front())
    }

    fn send_eos(&mut self) -> Result<()> {
        while let Some(frame) = self.dec.flush() {
            let poc = frame.pic_order_cnt;
            let frame_pts = frame.pts;
            self.push_frame(frame, poc, frame_pts)?;
        }
        self.drain_all_pending();
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.dec = Decoder::new();
        self.queued.clear();
        self.pending.clear();
        self.gop_count = 0;
        self.in_irap_picture = false;
        self.hvcc_length_size = None;
        self.pending_pts = None;
        self.frame_duration_us = 0;

        if matches!(self.nal_format, NalFormat::Hvcc { .. }) {
            self.prime_with_hvcc_extradata()?;
        }
        Ok(())
    }

    fn requested_output_format(&self) -> VideoOutputFormat {
        match self.opts.output_format {
            // P010 rejected at construction; returns Yuv420 (U16 for 10+ bit)
            VideoOutputFormat::P010 => VideoOutputFormat::Yuv420,
            VideoOutputFormat::Nv12 => VideoOutputFormat::Nv12,
            VideoOutputFormat::Native | VideoOutputFormat::Yuv420 => VideoOutputFormat::Yuv420,
        }
    }

    fn set_frame_duration_micros(&mut self, us: u64) {
        self.frame_duration_us = us;
    }
}
