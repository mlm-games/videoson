//! Colour signalling read out of decoder extradata.
//!
//! Hardware decoders on every platform discard colour: MediaCodec,
//! WebCodecs and VideoToolbox all decode without reporting it. A caller
//! driving a hardware decoder therefore has to read the CICP codes itself
//! and pass them down, which is what these helpers are for.
//!
//! Every function returns `None` when the stream signals no
//! `video_signal_type`, or signals one with
//! `colour_description_present_flag == 0`. H.264 §E.3.1 and H.265 §E.3.1
//! both define those as *unspecified*, which is not the same as
//! "BT.601" — a caller wanting a concrete matrix must choose its own
//! documented fallback rather than have one chosen here.

use videoson_core::{CodecType, ColorInfo};

/// Read colour from an `AVCDecoderConfigurationRecord` (ISO/IEC 14496-15
/// §5.3.3.1), taking the VUI of the first carried SPS.
#[cfg(feature = "h264")]
pub fn avcc_color(extradata: &[u8]) -> Option<ColorInfo> {
    let record = strip_box(extradata, b"avcC")?;
    if record.len() < 6 || record[0] != 1 {
        return None;
    }
    let num_sps = usize::from(record[5] & 0x1F);
    let mut pos = 6;
    for _ in 0..num_sps {
        let len = usize::from(u16::from_be_bytes([
            *record.get(pos)?,
            *record.get(pos + 1)?,
        ]));
        pos += 2;
        let nal = record.get(pos..pos.checked_add(len)?)?;
        pos += len;
        if nal.first()? & 0x1F != 7 {
            continue;
        }
        let rbsp = ebsp_to_rbsp(nal.get(1..)?);
        return videoson_codec_h264::cicp_from_sps_rbsp(&rbsp);
    }
    None
}

/// Read colour from an `HEVCDecoderConfigurationRecord` (ISO/IEC 14496-15
/// §8.3.3.1.2), taking the VUI of the first carried SPS (NAL type 33).
#[cfg(feature = "h265")]
pub fn hvcc_color(extradata: &[u8]) -> Option<ColorInfo> {
    let record = strip_box(extradata, b"hvcC")?;
    let parsed = oxideav_h265::hvcc::parse_hvcc(record).ok()?;
    videoson_codec_h265::cicp_from_nal_units(&parsed.nal_units)
}

/// Read colour from `extradata`, choosing the record layout from `codec`.
///
/// `extradata` may be bare or wrapped in its ISO-BMFF box header.
#[cfg(any(feature = "h264", feature = "h265"))]
pub fn extradata_color(codec: CodecType, extradata: &[u8]) -> Option<ColorInfo> {
    match codec {
        #[cfg(feature = "h264")]
        CodecType::H264 => avcc_color(extradata),
        #[cfg(feature = "h265")]
        CodecType::H265 => hvcc_color(extradata),
        _ => None,
    }
}

/// Drop a leading 8-byte box header (4-byte size + 4-byte type) when the
/// extradata is wrapped in one, as it is inside an `stsd` entry.
#[cfg(any(feature = "h264", feature = "h265"))]
fn strip_box<'a>(extradata: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    if extradata.len() >= 8 && &extradata[4..8] == fourcc {
        Some(&extradata[8..])
    } else {
        Some(extradata)
    }
}

/// Strip `emulation_prevention_three_byte` per §7.4.1.1.
#[cfg(feature = "h264")]
fn ebsp_to_rbsp(ebsp: &[u8]) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec::Vec::with_capacity(ebsp.len());
    let mut zeros = 0u32;
    for &b in ebsp {
        if zeros >= 2 && b == 0x03 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}
