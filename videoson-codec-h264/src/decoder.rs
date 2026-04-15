// videoson-codec-h264/src/decoder.rs
extern crate alloc;

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use videoson_common::{annexb_nals, avcc_nals, ebsp_to_rbsp, BitstreamError, BitstreamResult};
use videoson_core::{
    CodecType, NalFormat, Packet, Result, VideoCodecParams, VideoDecoder, VideoDecoderOptions,
    VideoFrame, VideosonError,
};

use crate::pps::Pps;
use crate::slice::decode_idr_ipcm_slice;
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

#[derive(Debug)]
pub struct H264Decoder {
    params: VideoCodecParams,
    _opts: VideoDecoderOptions,

    nal_format: NalFormat,

    ps: ParamSets,

    rbsp_scratch: Vec<u8>,
    out: VecDeque<VideoFrame>,
}

impl H264Decoder {
    fn iter_nals<'a>(
        &'a self,
        data: &'a [u8],
    ) -> Box<dyn Iterator<Item = BitstreamResult<videoson_common::NalUnitRef<'a>>> + 'a> {
        match self.nal_format {
            NalFormat::AnnexB => Box::new(annexb_nals(data)),
            NalFormat::Avcc { nal_len_size } => Box::new(avcc_nals(data, nal_len_size)),
            _ => Box::new(core::iter::empty()),
        }
    }

    fn handle_nal(&mut self, n: videoson_common::NalUnitRef<'_>) -> Result<()> {
        let rbsp = ebsp_to_rbsp(n.payload_ebsp, &mut self.rbsp_scratch);

        match n.header.nal_unit_type {
            6 => Ok(()), // SEI ignored
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
                let sh = bs(crate::slice::parse_slice_header_rbsp(rbsp, &self.ps))?;

                let pps = self.ps.get_pps(sh.pps_id)?;
                if pps.entropy_coding_mode_flag {
                    return Err(VideosonError::Unsupported(
                        "CABAC not supported (entropy_coding_mode_flag=1)",
                    ));
                }

                let frame = decode_idr_ipcm_slice(rbsp, &self.ps, &sh)?;
                self.out.push_back(frame);
                Ok(())
            }
            1 => Err(VideosonError::Unsupported(
                "non-IDR slice not supported in M0",
            )),
            9 => Ok(()),            // AUD ignored
            10 | 11 | 12 => Ok(()), // EOS/filler ignored
            _ => Ok(()),            // other NAL types ignored
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
    }
}
