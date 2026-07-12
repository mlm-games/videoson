use alloc::vec;
use alloc::vec::Vec;

use videoson_core::{Demuxer, Packet, TimeBase, VideoCodecParams};

use crate::header::{IVF_FILE_HEADER_LEN, IVF_FRAME_HEADER_LEN, IvfFileHeader, IvfFrameHeader};

fn is_keyframe(codec_type: videoson_core::CodecType, payload: &[u8]) -> bool {
    if payload.is_empty() {
        return false;
    }
    match codec_type {
        // VP8/VP9: first bit of payload indicates keyframe (0=key, 1=inter).
        // VP8 frame tag: bits [0] show_frame=0 means keyframe.
        // VP9 frame marker: byte 0 bit 0 = 0 means keyframe.
        videoson_core::CodecType::VP8 | videoson_core::CodecType::VP9 => (payload[0] & 0x01) == 0,
        // AV1: first bit of OBU header. 0 = sequence header/key frame.
        // Approximate: check for temporal delimiter or sequence header OBU.
        videoson_core::CodecType::AV1 => (payload[0] >> 7) == 0,
        _ => false,
    }
}

pub struct IvfDemuxer {
    data: Vec<u8>,
    pos: usize,
    file_header: IvfFileHeader,
    header_len: usize,
    track_id: u32,
    frame_index: u64,
    tracks: Vec<videoson_core::Track>,
}

impl IvfDemuxer {
    pub fn new(data: Vec<u8>) -> core::result::Result<Self, videoson_core::VideosonError> {
        if data.len() < IVF_FILE_HEADER_LEN {
            return Err(videoson_core::VideosonError::NeedMoreData);
        }
        let file_header = IvfFileHeader::parse(&data[..IVF_FILE_HEADER_LEN])?;
        let header_len = file_header.header_len as usize;

        if data.len() < header_len {
            return Err(videoson_core::VideosonError::NeedMoreData);
        }

        let codec = file_header
            .codec
            .to_codec_type()
            .ok_or(videoson_core::VideosonError::Unsupported("IVF codec"))?;

        let mut codec_params = VideoCodecParams::new(codec);
        codec_params.coded_width = file_header.width as u32;
        codec_params.coded_height = file_header.height as u32;

        let time_base = TimeBase::new(file_header.fps_den, file_header.fps_num.max(1));

        let tracks = vec![videoson_core::Track {
            id: 0,
            codec_params,
            time_base: Some(time_base),
        }];

        Ok(Self {
            data,
            pos: header_len,
            file_header,
            header_len,
            track_id: 0,
            frame_index: 0,
            tracks,
        })
    }

    pub fn file_header(&self) -> &IvfFileHeader {
        &self.file_header
    }

    pub fn codec_type(&self) -> Option<videoson_core::CodecType> {
        self.file_header.codec.to_codec_type()
    }

    fn read_next_packet(
        &mut self,
    ) -> core::result::Result<Option<Packet>, videoson_core::VideosonError> {
        let remaining = self.data.len().saturating_sub(self.pos);
        if remaining == 0 {
            return Ok(None);
        }
        if remaining < IVF_FRAME_HEADER_LEN {
            return Err(videoson_core::VideosonError::InvalidData(
                "IVF: truncated frame",
            ));
        }

        let fh_buf = &self.data[self.pos..self.pos + IVF_FRAME_HEADER_LEN];
        let frame_hdr = IvfFrameHeader::parse(fh_buf)?;
        self.pos += IVF_FRAME_HEADER_LEN;

        let payload_len = frame_hdr.frame_size as usize;
        if self.pos + payload_len > self.data.len() {
            return Err(videoson_core::VideosonError::InvalidData(
                "IVF: truncated payload",
            ));
        }

        let payload = self.data[self.pos..self.pos + payload_len].to_vec();
        self.pos += payload_len;

        let codec_type = self.file_header.codec.to_codec_type();
        let is_sync = codec_type.map_or(false, |ct| is_keyframe(ct, &payload));

        let mut pkt = Packet::new(self.track_id, payload);
        pkt.pts = Some(frame_hdr.timestamp as i64);
        pkt.dts = Some(frame_hdr.timestamp as i64);
        pkt.is_sync = is_sync;

        self.frame_index += 1;
        Ok(Some(pkt))
    }

    pub fn seek_to_ts(
        &mut self,
        target_ts: u64,
    ) -> core::result::Result<(), videoson_core::VideosonError> {
        self.pos = self.header_len;
        self.frame_index = 0;

        loop {
            let remaining = self.data.len().saturating_sub(self.pos);
            if remaining < IVF_FRAME_HEADER_LEN {
                break;
            }

            let fh_buf = &self.data[self.pos..self.pos + IVF_FRAME_HEADER_LEN];
            let frame_hdr = IvfFrameHeader::parse(fh_buf)?;

            if frame_hdr.timestamp >= target_ts {
                break;
            }

            let skip = IVF_FRAME_HEADER_LEN + frame_hdr.frame_size as usize;
            if self.pos + skip > self.data.len() {
                break;
            }
            self.pos += skip;
            self.frame_index += 1;
        }
        Ok(())
    }

    pub fn bytes_consumed(&self) -> usize {
        self.pos
    }

    pub fn total_bytes(&self) -> usize {
        self.data.len()
    }
}

impl Demuxer for IvfDemuxer {
    fn tracks(&self) -> &[videoson_core::Track] {
        &self.tracks
    }

    fn next_packet(&mut self) -> videoson_core::Result<Option<Packet>> {
        self.read_next_packet()
    }
}
