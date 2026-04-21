use alloc::vec::Vec;

use videoson_core::Packet;

use crate::header::{IVF_FILE_HEADER_LEN, IVF_FRAME_HEADER_LEN, IvfFileHeader, IvfFrameHeader};

pub struct IvfDemuxer {
    data: Vec<u8>,
    pos: usize,
    file_header: IvfFileHeader,
    track_id: u32,
    frame_index: u64,
}

impl IvfDemuxer {
    pub fn new(data: Vec<u8>) -> core::result::Result<Self, videoson_core::VideosonError> {
        if data.len() < IVF_FILE_HEADER_LEN {
            return Err(videoson_core::VideosonError::NeedMoreData);
        }
        let file_header = IvfFileHeader::parse(&data[..IVF_FILE_HEADER_LEN])?;
        Ok(Self {
            data,
            pos: IVF_FILE_HEADER_LEN,
            file_header,
            track_id: 0,
            frame_index: 0,
        })
    }

    pub fn file_header(&self) -> &IvfFileHeader {
        &self.file_header
    }

    pub fn codec_type(&self) -> Option<videoson_core::CodecType> {
        self.file_header.codec.to_codec_type()
    }

    pub fn next_packet(
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

        let mut pkt = Packet::new(self.track_id, payload);
        pkt.pts = Some(frame_hdr.timestamp as i64);
        pkt.dts = Some(frame_hdr.timestamp as i64);

        self.frame_index += 1;
        Ok(Some(pkt))
    }

    pub fn seek_to_ts(
        &mut self,
        target_ts: u64,
    ) -> core::result::Result<(), videoson_core::VideosonError> {
        self.pos = IVF_FILE_HEADER_LEN;
        self.frame_index = 0;

        loop {
            let remaining = self.data.len().saturating_sub(self.pos);
            if remaining < IVF_FRAME_HEADER_LEN {
                break;
            }

            let fh_buf = &self.data[self.pos..self.pos + IVF_FRAME_HEADER_LEN];
            let frame_hdr = IvfFrameHeader::parse(fh_buf)?;

            if frame_hdr.timestamp > target_ts {
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
