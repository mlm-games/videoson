// videoson-codec-h264/src/decoder.rs
extern crate alloc;

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use videoson_common::{annexb_nals, avcc_nals, ebsp_to_rbsp, BitstreamError, BitstreamResult};
use videoson_core::{
    CodecType, NalFormat, Packet, PlaneData, Result, VideoCodecParams, VideoDecoder,
    VideoDecoderOptions, VideoFrame, VideoFramePlanes, VideoPlane, VideosonError,
};

use crate::pps::Pps;
use crate::sps::Sps;

pub(crate) struct ParamSets {
    sps: [Option<Sps>; 32],
    pps: [Option<Pps>; 256],
}

impl core::fmt::Debug for ParamSets {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ParamSets").finish()
    }
}

impl ParamSets {
    fn new() -> Self {
        Self {
            sps: core::array::from_fn(|_| None),
            pps: core::array::from_fn(|_| None),
        }
    }

    fn put_sps(&mut self, sps: Sps) {
        let id = sps.sps_id as usize;
        if id < self.sps.len() {
            self.sps[id] = Some(sps);
        }
    }

    fn put_pps(&mut self, pps: Pps) {
        let id = pps.pps_id as usize;
        if id < self.pps.len() {
            self.pps[id] = Some(pps);
        }
    }

    pub(crate) fn get_pps(&self, pps_id: u32) -> core::result::Result<&Pps, VideosonError> {
        let idx = pps_id as usize;
        self.pps
            .get(idx)
            .and_then(|x| x.as_ref())
            .ok_or(VideosonError::InvalidData("missing PPS"))
    }

    pub(crate) fn get_sps(&self, sps_id: u32) -> core::result::Result<&Sps, VideosonError> {
        let idx = sps_id as usize;
        self.sps
            .get(idx)
            .and_then(|x| x.as_ref())
            .ok_or(VideosonError::InvalidData("missing SPS"))
    }
}

fn map_bs_err(e: BitstreamError) -> VideosonError {
    match e {
        BitstreamError::Eof => VideosonError::NeedMoreData,
        BitstreamError::Invalid(s) => VideosonError::InvalidData(s),
        BitstreamError::Message(s) => VideosonError::Message(s),
        _ => VideosonError::InvalidData("unknown bitstream error"),
    }
}

fn bs<T>(r: BitstreamResult<T>) -> Result<T> {
    r.map_err(map_bs_err)
}

#[derive(Debug, Clone)]
pub(crate) enum PendingPlanes {
    Mono8 {
        y: Vec<u8>,
    },
    Yuv4208 {
        y: Vec<u8>,
        u: Vec<u8>,
        v: Vec<u8>,
    },
    Mono16 {
        y: Vec<u16>,
    },
    Yuv42016 {
        y: Vec<u16>,
        u: Vec<u16>,
        v: Vec<u16>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct PendingPic {
    pub(crate) frame_num: u32,
    pub(crate) idr_pic_id: u32,
    pub(crate) pic_order_cnt_lsb: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bit_depth: u8,
    pub(crate) chroma_format_idc: u32,
    pub(crate) mbs_w: usize,
    pub(crate) mbs_h: usize,
    pub(crate) y_stride: usize,
    pub(crate) chroma_w: usize,
    pub(crate) chroma_h: usize,
    pub(crate) uv_stride: usize,
    pub(crate) planes: PendingPlanes,
    pub(crate) mb_types: Vec<u8>,
    pub(crate) filled: Vec<bool>,
    pub(crate) filled_count: usize,
    pub(crate) intra4x4_modes: Vec<u8>,
    pub(crate) nz_y: Vec<u8>,
    pub(crate) nz_u: Vec<u8>,
    pub(crate) nz_v: Vec<u8>,
    pub(crate) nz_y_dc: Vec<u8>,
}

impl PendingPic {
    pub(crate) fn new_from_sps(
        sps: &Sps,
        frame_num: u32,
        idr_pic_id: u32,
        pic_order_cnt_lsb: u32,
    ) -> Result<Self> {
        let (width, height) = sps.display_dimensions();
        let width_us = width as usize;
        let height_us = height as usize;

        let bit_depth = sps.bit_depth_luma;
        let chroma_format_idc = sps.chroma_format_idc;

        if chroma_format_idc != 0 && chroma_format_idc != 1 {
            return Err(VideosonError::Unsupported(
                "only chroma_format_idc 0 (mono) and 1 (4:2:0) supported in M0",
            ));
        }

        let mbs_w = sps.mbs_width() as usize;
        let mbs_h = sps.mbs_height() as usize;
        let total_mbs = mbs_w * mbs_h;

        let y_stride = width_us;
        let chroma_w = (width_us + 1) / 2;
        let chroma_h = (height_us + 1) / 2;
        let uv_stride = chroma_w;

        let planes = if bit_depth <= 8 {
            let y = vec![0u8; y_stride * height_us];
            if chroma_format_idc == 0 {
                PendingPlanes::Mono8 { y }
            } else {
                let u = vec![128u8; uv_stride * chroma_h];
                let v = vec![128u8; uv_stride * chroma_h];
                PendingPlanes::Yuv4208 { y, u, v }
            }
        } else {
            let y = vec![0u16; y_stride * height_us];
            if chroma_format_idc == 0 {
                PendingPlanes::Mono16 { y }
            } else {
                let mid = 1u16 << (bit_depth - 1);
                let u = vec![mid; uv_stride * chroma_h];
                let v = vec![mid; uv_stride * chroma_h];
                PendingPlanes::Yuv42016 { y, u, v }
            }
        };

        Ok(Self {
            frame_num,
            idr_pic_id,
            pic_order_cnt_lsb,
            width,
            height,
            bit_depth,
            chroma_format_idc,
            mbs_w,
            mbs_h,
            y_stride,
            chroma_w,
            chroma_h,
            uv_stride,
            planes,
            mb_types: vec![0u8; total_mbs],
            filled: vec![false; total_mbs],
            filled_count: 0,
            intra4x4_modes: vec![2u8; total_mbs * 16],
            nz_y: vec![0u8; total_mbs * 16],
            nz_u: vec![0u8; total_mbs * 4],
            nz_v: vec![0u8; total_mbs * 4],
            nz_y_dc: vec![0u8; total_mbs],
        })
    }

    #[inline]
    pub(crate) fn total_mbs(&self) -> usize {
        self.mbs_w * self.mbs_h
    }

    #[inline]
    pub(crate) fn mark_mb_decoded(&mut self, mb_addr: usize, mb_type: u8) {
        if mb_addr < self.mb_types.len() {
            self.mb_types[mb_addr] = mb_type;
            if !self.filled[mb_addr] {
                self.filled[mb_addr] = true;
                self.filled_count += 1;
            }
        }
    }

    #[inline]
    pub(crate) fn neighbor_mb_type(&self, mb_addr: usize) -> Option<u8> {
        if mb_addr >= self.mb_types.len() || !self.filled[mb_addr] {
            None
        } else {
            Some(self.mb_types[mb_addr])
        }
    }

    #[inline]
    pub(crate) fn idx_y4(&self, mb_addr: usize, blk: usize) -> usize {
        mb_addr * 16 + blk
    }

    #[inline]
    pub(crate) fn idx_c4(&self, mb_addr: usize, blk: usize) -> usize {
        mb_addr * 4 + blk
    }

    #[inline]
    pub(crate) fn is_complete(&self) -> bool {
        self.filled_count == self.total_mbs()
    }

    pub(crate) fn into_frame(self) -> VideoFrame {
        match self.planes {
            PendingPlanes::Mono8 { y } => VideoFrame {
                width: self.width,
                height: self.height,
                planes: VideoFramePlanes::Mono,
                pixfmt: videoson_core::PixelFormat::Gray,
                bit_depth: self.bit_depth,
                plane_data: vec![VideoPlane {
                    stride: self.y_stride,
                    data: PlaneData::U8(y),
                }],
            },
            PendingPlanes::Yuv4208 { y, u, v } => VideoFrame {
                width: self.width,
                height: self.height,
                planes: VideoFramePlanes::Yuv420,
                pixfmt: videoson_core::PixelFormat::Yuv420,
                bit_depth: self.bit_depth,
                plane_data: vec![
                    VideoPlane {
                        stride: self.y_stride,
                        data: PlaneData::U8(y),
                    },
                    VideoPlane {
                        stride: self.uv_stride,
                        data: PlaneData::U8(u),
                    },
                    VideoPlane {
                        stride: self.uv_stride,
                        data: PlaneData::U8(v),
                    },
                ],
            },
            PendingPlanes::Mono16 { y } => VideoFrame {
                width: self.width,
                height: self.height,
                planes: VideoFramePlanes::Mono,
                pixfmt: videoson_core::PixelFormat::Gray,
                bit_depth: self.bit_depth,
                plane_data: vec![VideoPlane {
                    stride: self.y_stride,
                    data: PlaneData::U16(y),
                }],
            },
            PendingPlanes::Yuv42016 { y, u, v } => VideoFrame {
                width: self.width,
                height: self.height,
                planes: VideoFramePlanes::Yuv420,
                pixfmt: videoson_core::PixelFormat::Yuv420,
                bit_depth: self.bit_depth,
                plane_data: vec![
                    VideoPlane {
                        stride: self.y_stride,
                        data: PlaneData::U16(y),
                    },
                    VideoPlane {
                        stride: self.uv_stride,
                        data: PlaneData::U16(u),
                    },
                    VideoPlane {
                        stride: self.uv_stride,
                        data: PlaneData::U16(v),
                    },
                ],
            },
        }
    }
}

#[derive(Debug)]
pub struct H264Decoder {
    params: VideoCodecParams,
    _opts: VideoDecoderOptions,
    nal_format: NalFormat,
    ps: ParamSets,
    rbsp_scratch: Vec<u8>,
    out: VecDeque<VideoFrame>,
    pending: Option<PendingPic>,
}

impl H264Decoder {
    fn handle_nal(&mut self, n: videoson_common::NalUnitRef<'_>) -> Result<()> {
        let rbsp = ebsp_to_rbsp(n.payload_ebsp, &mut self.rbsp_scratch);

        match n.header.nal_unit_type {
            6 => Ok(()),
            7 => {
                let sps = bs(crate::sps::parse_sps_rbsp(rbsp))?;
                self.ps.put_sps(sps);
                Ok(())
            }
            8 => {
                let pps = bs(crate::pps::parse_pps_rbsp(rbsp))?;
                self.ps.put_pps(pps);
                Ok(())
            }
            5 => {
                let (sh, header_bits) = bs(crate::slice::parse_slice_header_rbsp(rbsp, &self.ps))?;
                let pps = self.ps.get_pps(sh.pps_id)?;
                let sps = self.ps.get_sps(pps.sps_id)?;

                let idr_pic_id = sh
                    .idr_pic_id
                    .ok_or(VideosonError::InvalidData("missing idr_pic_id"))?;
                let poc = sh
                    .pic_order_cnt_lsb
                    .ok_or(VideosonError::InvalidData("missing pic_order_cnt_lsb"))?;

                if let Some(p) = &self.pending {
                    let same = p.frame_num == sh.frame_num
                        && p.idr_pic_id == idr_pic_id
                        && p.pic_order_cnt_lsb == poc;
                    if !same {
                        if p.is_complete() {
                            let frame = self.pending.take().unwrap().into_frame();
                            self.out.push_back(frame);
                        } else {
                            return Err(VideosonError::Unsupported(
                                "new picture started before previous picture was complete",
                            ));
                        }
                    }
                }

                if self.pending.is_none() {
                    self.pending = Some(PendingPic::new_from_sps(
                        sps,
                        sh.frame_num,
                        idr_pic_id,
                        poc,
                    )?);
                }

                {
                    let pic = self.pending.as_mut().unwrap();
                    crate::slice::decode_idr_slice_into_pic(
                        rbsp,
                        header_bits,
                        &self.ps,
                        &sh,
                        pps,
                        pic,
                    )?;
                }

                if self
                    .pending
                    .as_ref()
                    .map(|p| p.is_complete())
                    .unwrap_or(false)
                {
                    let frame = self.pending.take().unwrap().into_frame();
                    self.out.push_back(frame);
                }

                Ok(())
            }
            1 => Err(VideosonError::Unsupported(
                "non-IDR slice not supported in M0",
            )),
            9 => Ok(()),
            10 | 11 | 12 => Ok(()),
            _ => Ok(()),
        }
    }
}

impl VideoDecoder for H264Decoder {
    fn try_new(params: &VideoCodecParams, opts: &VideoDecoderOptions) -> Result<Self> {
        if params.codec != CodecType::H264 {
            return Err(VideosonError::InvalidData("params.codec is not H264"));
        }

        let nal_format = params.nal_format.unwrap_or(NalFormat::AnnexB);

        Ok(Self {
            params: params.clone(),
            _opts: *opts,
            nal_format,
            ps: ParamSets::new(),
            rbsp_scratch: Vec::new(),
            out: VecDeque::new(),
            pending: None,
        })
    }

    fn codec_params(&self) -> &VideoCodecParams {
        &self.params
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        let data = packet.data.clone();
        let nal_format = self.nal_format;

        let nals: core::result::Result<Vec<_>, _> = match nal_format {
            NalFormat::AnnexB => {
                let mut nals_vec = Vec::new();
                for nal_result in annexb_nals(&data) {
                    nals_vec.push(nal_result.map_err(map_bs_err)?);
                }
                Ok(nals_vec)
            }
            NalFormat::Avcc { nal_len_size } => {
                let mut nals_vec = Vec::new();
                for nal_result in avcc_nals(&data, nal_len_size) {
                    nals_vec.push(nal_result.map_err(map_bs_err)?);
                }
                Ok(nals_vec)
            }
            _ => Ok(Vec::new()),
        };
        let nals = nals?;

        for nal in nals {
            self.handle_nal(nal)?;
        }
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>> {
        Ok(self.out.pop_front())
    }

    fn reset(&mut self) {
        self.ps = ParamSets::new();
        self.rbsp_scratch.clear();
        self.out.clear();
        self.pending = None;
    }
}
