use alloc::vec;
use alloc::vec::Vec;

use videoson_core::{Demuxer, Packet, TimeBase, VideoCodecParams};

use oxideav_bitstream::av1::{
    parse_frame_header, parse_sequence_header, read_obu,
    OBU_FRAME, OBU_FRAME_HEADER, OBU_REDUNDANT_FRAME_HEADER, OBU_SEQUENCE_HEADER,
};

use crate::header::{IVF_FILE_HEADER_LEN, IVF_FRAME_HEADER_LEN, IvfFileHeader, IvfFrameHeader};

/// Minimal VP8 frame tag parser for keyframe detection.
fn vp8_is_keyframe(payload: &[u8]) -> bool {
    // VP8 frame tag byte 0 bit 0 = 0 means keyframe.
    if payload.is_empty() || (payload[0] & 0x01) != 0 {
        return false;
    }
    // Validate keyframe start code: bytes 3-5 must be 0x9d 0x01 0x2a
    if payload.len() < 6 {
        return false;
    }
    payload[3] == 0x9d && payload[4] == 0x01 && payload[5] == 0x2a
}

/// VP9 keyframe detection via the uncompressed header.
/// Frame byte 0 bits: [0..1] frame_marker (must be 0b10),
/// [2..3] profile_low, [4] profile_high, [5] show_existing_frame,
/// [6] error_resilient_mode, [7] frame_type (0=keyframe).
fn vp9_is_keyframe(payload: &[u8]) -> bool {
    if payload.len() < 2 {
        return false;
    }
    let b0 = payload[0];
    let b1 = payload[1];

    // frame_marker must be 0b10 in bits [1..0]
    if b0 & 0x03 != 0x02 {
        return false;
    }

    let frame_type = (b0 >> 7) & 1;
    if frame_type != 0 {
        // inter frame - not a keyframe
        return false;
    }

    let show_existing_frame = (b0 >> 5) & 1;
    if show_existing_frame == 1 {
        // show_existing_frame refers to the existing frame, it's not a keyframe
        return false;
    }

    let _error_resilient_mode = (b0 >> 6) & 1;

    // profile = ((b0 >> 4) & 1) << 1 | ((b0 >> 2) & 0x03)
    let profile = (((b0 >> 4) & 1) << 1) | ((b0 >> 2) & 0x03);

    if profile == 3 {
        // Skip 4 extra profile bits at byte 1 bits [3..0]
        let _ = (b1 >> 4) & 0x0f;
    }

    // sync code: bytes should contain 0x49 0x83 0x42 after the frame marker
    // Starting at byte 1 for profiles 0-2, byte 2 for profile 3
    let sync_offset = if profile == 3 { 2 } else { 1 };
    if payload.len() < sync_offset + 3 {
        return false;
    }
    payload[sync_offset] == 0x49
        && payload[sync_offset + 1] == 0x83
        && payload[sync_offset + 2] == 0x42
}

/// AV1 keyframe detection.
///
/// parses the sequence header then checks whether the first frame OBU
/// is a KEY_FRAME via `parse_frame_header`.
fn av1_is_keyframe(payload: &[u8]) -> bool {
    let mut offset = 0;
    let mut seq_header = None;

    while offset < payload.len() {
        let (hdr, payload_start, payload_end, next) = match read_obu(payload, offset) {
            Ok(v) => v,
            Err(_) => return false,
        };

        match hdr.obu_type {
            OBU_SEQUENCE_HEADER => {
                if seq_header.is_none() {
                    seq_header = parse_sequence_header(&payload[payload_start..payload_end]).ok();
                }
            }
            OBU_FRAME | OBU_FRAME_HEADER | OBU_REDUNDANT_FRAME_HEADER => {
                return seq_header.as_ref().is_some_and(|seq| {
                    parse_frame_header(&payload[payload_start..payload_end], seq).is_ok()
                });
            }
            _ => {}
        }

        offset = next;
    }
    false
}

fn is_keyframe(codec_type: videoson_core::CodecType, payload: &[u8]) -> bool {
    if payload.is_empty() {
        return false;
    }
    match codec_type {
        videoson_core::CodecType::VP8 => vp8_is_keyframe(payload),
        videoson_core::CodecType::VP9 => vp9_is_keyframe(payload),
        videoson_core::CodecType::AV1 => av1_is_keyframe(payload),
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

        let fh_end = self.pos.checked_add(IVF_FRAME_HEADER_LEN).ok_or(
            videoson_core::VideosonError::InvalidData("IVF: frame header overflow"),
        )?;
        let fh_buf = &self.data[self.pos..fh_end];
        let frame_hdr = IvfFrameHeader::parse(fh_buf)?;
        self.pos = fh_end;

        let payload_len = frame_hdr.frame_size as usize;
        let payload_end = self.pos.checked_add(payload_len).ok_or(
            videoson_core::VideosonError::InvalidData("IVF: payload overflow"),
        )?;
        if payload_end > self.data.len() {
            return Err(videoson_core::VideosonError::InvalidData(
                "IVF: truncated payload",
            ));
        }

        let payload = self.data[self.pos..payload_end].to_vec();
        self.pos = payload_end;

        let codec_type = self.file_header.codec.to_codec_type();
        let is_sync = codec_type.map_or(false, |ct| is_keyframe(ct, &payload));

        let mut pkt = Packet::new(self.track_id, payload);
        let ts = i64::try_from(frame_hdr.timestamp)
            .map_err(|_| videoson_core::VideosonError::InvalidData("IVF: timestamp exceeds i64"))?;
        pkt.pts = Some(ts);
        pkt.dts = Some(ts);
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

            let fh_end = self.pos.checked_add(IVF_FRAME_HEADER_LEN).ok_or(
                videoson_core::VideosonError::InvalidData("IVF: seek overflow"),
            )?;
            let fh_buf = &self.data[self.pos..fh_end];
            let frame_hdr = IvfFrameHeader::parse(fh_buf)?;

            if frame_hdr.timestamp >= target_ts {
                break;
            }

            let frame_size = frame_hdr.frame_size as usize;
            let skip = IVF_FRAME_HEADER_LEN.checked_add(frame_size).ok_or(
                videoson_core::VideosonError::InvalidData("IVF: seek skip overflow"),
            )?;
            let next_pos = self.pos.checked_add(skip).ok_or(
                videoson_core::VideosonError::InvalidData("IVF: seek position overflow"),
            )?;
            if next_pos > self.data.len() {
                break;
            }
            self.pos = next_pos;
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
