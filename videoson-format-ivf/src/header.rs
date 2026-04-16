pub const IVF_SIGNATURE: &[u8; 4] = b"DKIF";
pub const IVF_FILE_HEADER_LEN: usize = 32;
pub const IVF_FRAME_HEADER_LEN: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IvfCodec {
    Av1,
    Vp9,
    Vp8,
    Unknown([u8; 4]),
}

impl IvfCodec {
    pub fn from_fourcc(cc: &[u8; 4]) -> Self {
        if cc == b"AV01" {
            return IvfCodec::Av1;
        }
        if cc == b"VP90" {
            return IvfCodec::Vp9;
        }
        if cc == b"VP80" {
            return IvfCodec::Vp8;
        }
        IvfCodec::Unknown([cc[0], cc[1], cc[2], cc[3]])
    }

    pub fn to_codec_type(self) -> Option<videoson_core::CodecType> {
        match self {
            IvfCodec::Av1 => Some(videoson_core::CodecType::AV1),
            IvfCodec::Vp9 => Some(videoson_core::CodecType::VP9),
            IvfCodec::Vp8 => Some(videoson_core::CodecType::VP8),
            IvfCodec::Unknown(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IvfFileHeader {
    pub codec: IvfCodec,
    pub width: u16,
    pub height: u16,
    pub fps_num: u32,
    pub fps_den: u32,
    pub frame_cnt: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct IvfFrameHeader {
    pub frame_size: u32,
    pub timestamp: u64,
}

impl IvfFileHeader {
    pub fn parse(buf: &[u8]) -> core::result::Result<Self, videoson_core::VideosonError> {
        if buf.len() < IVF_FILE_HEADER_LEN {
            return Err(videoson_core::VideosonError::NeedMoreData);
        }
        if buf[0] != b'D' || buf[1] != b'K' || buf[2] != b'I' || buf[3] != b'F' {
            return Err(videoson_core::VideosonError::InvalidData(
                "IVF: bad signature",
            ));
        }
        let version = u16::from_le_bytes([buf[4], buf[5]]);
        let header_len = u16::from_le_bytes([buf[6], buf[7]]);
        if version != 0 {
            return Err(videoson_core::VideosonError::InvalidData(
                "IVF: unsupported version",
            ));
        }
        if header_len != 32 {
            return Err(videoson_core::VideosonError::InvalidData(
                "IVF: unexpected header_len",
            ));
        }

        let fourcc: [u8; 4] = [buf[8], buf[9], buf[10], buf[11]];
        let width = u16::from_le_bytes([buf[12], buf[13]]);
        let height = u16::from_le_bytes([buf[14], buf[15]]);
        let fps_num = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
        let fps_den = u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]);
        let frame_cnt = u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]);

        Ok(Self {
            codec: IvfCodec::from_fourcc(&fourcc),
            width,
            height,
            fps_num,
            fps_den,
            frame_cnt,
        })
    }
}

impl IvfFrameHeader {
    pub fn parse(buf: &[u8]) -> core::result::Result<Self, videoson_core::VideosonError> {
        if buf.len() < IVF_FRAME_HEADER_LEN {
            return Err(videoson_core::VideosonError::NeedMoreData);
        }
        let frame_size = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let timestamp_lo = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let timestamp_hi = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let timestamp = ((timestamp_hi as u64) << 32) | (timestamp_lo as u64);
        Ok(Self {
            frame_size,
            timestamp,
        })
    }
}
