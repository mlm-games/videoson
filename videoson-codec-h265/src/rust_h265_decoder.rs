//! It emits from in decode order, the player using this should reorder them according to pts

extern crate alloc;

use alloc::borrow::Cow;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

use rust_h265::{Decoder, NalUnit, NalUnitType, parse_annex_b};

use videoson_common::parse_hvcc_extradata;

use videoson_core::{
    CodecType, NalFormat, Packet, Result, VideoCodecParams, VideoDecoder, VideoDecoderOptions,
    VideoFrame, VideoOutputFormat, VideosonError, interleave_uv_nv12,
};

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
    hvcc_length_size: Option<u8>,
}

impl RustH265Decoder {
    fn map_err(e: rust_h265::DecodeError) -> VideosonError {
        VideosonError::Message(alloc::format!("H.265: {e:?}").into())
    }

    fn wants_nv12(&self) -> bool {
        matches!(self.opts.output_format, VideoOutputFormat::Nv12)
    }

    fn wants_p010(&self) -> bool {
        matches!(self.opts.output_format, VideoOutputFormat::P010)
    }

    fn push_frame(&mut self, f: rust_h265::decoder::Frame, pts: Option<i64>) -> Result<()> {
        let w = f.width as usize;
        let h = f.height as usize;
        let cw = (w + 1) / 2;
        let ch = (h + 1) / 2;
        let bd = f.bit_depth;

        let frame: VideoFrame = match bd {
            8 => self.make_frame_u8(f, w, h, cw, ch, pts)?,
            10 | 12 => self.make_frame_u16(f, w, h, cw, ch, bd, pts)?,
            _ => return Ok(()),
        };

        self.queued.push_back(frame);
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
        let y = f.y.as_u8().unwrap_or(&[]).to_vec();
        let u = f.u.as_u8().unwrap_or(&[]).to_vec();
        let v = f.v.as_u8().unwrap_or(&[]).to_vec();

        if self.wants_nv12() {
            let uv = interleave_uv_nv12(&u, cw, &v, cw, cw, ch)?;
            Ok(VideoFrame::new_nv12_u8(f.width, f.height, w, cw * 2, y, uv).with_pts(pts))
        } else {
            Ok(VideoFrame::new_yuv420_u8(f.width, f.height, w, cw, cw, y, u, v).with_pts(pts))
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
        let y = f.y.as_u16().unwrap_or(&[]).to_vec();
        let u = f.u.as_u16().unwrap_or(&[]).to_vec();
        let v = f.v.as_u16().unwrap_or(&[]).to_vec();

        if self.wants_p010() && bd == 10 {
            let uv_stride = cw * 2;
            let mut uv = Vec::with_capacity(cw * ch * 2);
            for row in 0..ch {
                for col in 0..cw {
                    uv.push(u.get(row * cw + col).copied().unwrap_or(0));
                    uv.push(v.get(row * cw + col).copied().unwrap_or(0));
                }
            }
            Ok(VideoFrame::new_p010_u16(f.width, f.height, w, uv_stride, y, uv).with_pts(pts))
        } else {
            Ok(VideoFrame::new_yuv420_u16(f.width, f.height, w, cw, cw, y, u, v, bd).with_pts(pts))
        }
    }

    fn feed_nal(&mut self, nal: &NalUnit<'_>, pts: Option<i64>) -> Result<()> {
        match self.dec.decode_nal(nal) {
            Ok(Some(frame)) => {
                self.push_frame(frame, pts)?;
            }
            Ok(None) => {}
            Err(e) => return Err(Self::map_err(e)),
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
        let extradata = &self.params.extradata;
        if extradata.is_empty() {
            return Ok(());
        }

        let payload: &[u8] = if extradata.len() >= 8 && &extradata[4..8] == b"hvcC" {
            &extradata[8..]
        } else {
            extradata.as_slice()
        };

        let cfg = parse_hvcc_extradata(payload)
            .map_err(|e| VideosonError::Message(alloc::format!("hvcC: {e:?}").into()))?;
        self.hvcc_length_size = Some(cfg.nal_len_size);

        let nal_list: Vec<Vec<u8>> = videoson_common::hvcc_nal_bytes(payload)
            .filter_map(|r| r.ok().map(|b| b.to_vec()))
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

        let nal_format = params.nal_format.unwrap_or(NalFormat::AnnexB);

        let mut me = Self {
            params: params.clone(),
            opts: *opts,
            nal_format,
            dec: Decoder::new(),
            queued: VecDeque::new(),
            hvcc_length_size: None,
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
        if let Some(frame) = self.dec.flush() {
            self.push_frame(frame, None)?;
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.dec = Decoder::new();
        self.queued.clear();
        self.hvcc_length_size = None;

        if matches!(self.nal_format, NalFormat::Hvcc { .. }) {
            let _ = self.prime_with_hvcc_extradata();
        }
    }

    fn output_format(&self) -> VideoOutputFormat {
        match self.opts.output_format {
            VideoOutputFormat::P010 => VideoOutputFormat::P010,
            VideoOutputFormat::Nv12 => VideoOutputFormat::Nv12,
            VideoOutputFormat::Native | VideoOutputFormat::Yuv420 => VideoOutputFormat::Yuv420,
        }
    }
}
