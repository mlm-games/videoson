extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use videoson_common::parse_avcc_extradata;
use videoson_core::{
    CodecType, Demuxer, NalFormat, Packet, TimeBase, Track, VideoCodecParams, VideosonError,
};

use matroska_demuxer::{Frame, MatroskaFile, TrackType};

#[cfg(feature = "std")]
pub struct MkvDemuxer {
    mkv: MatroskaFile<std::io::Cursor<Vec<u8>>>,
    tracks: Vec<Track>,
    track_map: BTreeMap<u64, u32>,
}

#[cfg(feature = "std")]
impl MkvDemuxer {
    pub fn new(data: Vec<u8>) -> videoson_core::Result<Self> {
        let cursor = std::io::Cursor::new(data);
        let mkv = MatroskaFile::open(cursor)
            .map_err(|_| VideosonError::InvalidData("mkv: open failed"))?;

        let ts_scale_ns = mkv.info().timestamp_scale().get();
        let ts_scale_u32: u32 = ts_scale_ns
            .try_into()
            .map_err(|_| VideosonError::Unsupported("mkv: TimestampScale > u32::MAX"))?;

        let time_base = Some(TimeBase::new(ts_scale_u32, 1_000_000_000));

        let mut tracks: Vec<Track> = Vec::new();
        let mut track_map: BTreeMap<u64, u32> = BTreeMap::new();

        for t in mkv
            .tracks()
            .iter()
            .filter(|t| t.track_type() == TrackType::Video)
        {
            let track_num_u64 = t.track_number().get();
            let track_id_u32: u32 = track_num_u64
                .try_into()
                .map_err(|_| VideosonError::Unsupported("mkv: track_number > u32::MAX"))?;

            let codec = match t.codec_id() {
                "V_MPEG4/ISO/AVC" => CodecType::H264,
                "V_AV1" => CodecType::AV1,
                "V_VP9" => CodecType::VP9,
                "V_VP8" => CodecType::VP8,
                _ => continue,
            };

            let mut params = VideoCodecParams::new(codec);

            if let Some(v) = t.video() {
                params.coded_width = v.pixel_width().get() as u32;
                params.coded_height = v.pixel_height().get() as u32;
            }

            if let Some(extra) = t.codec_private() {
                params.extradata = extra.to_vec();
            }

            if codec == CodecType::H264 {
                if params.extradata.is_empty() {
                    return Err(VideosonError::InvalidData("mkv: AVC missing CodecPrivate"));
                }
                let cfg = parse_avcc_extradata(&params.extradata)
                    .map_err(|_| VideosonError::InvalidData("mkv: bad AVC CodecPrivate"))?;
                params.nal_format = Some(NalFormat::Avcc {
                    nal_len_size: cfg.nal_len_size,
                });
            }

            tracks.push(Track {
                id: track_id_u32,
                codec_params: params,
                time_base,
            });
            track_map.insert(track_num_u64, track_id_u32);
        }

        if tracks.is_empty() {
            return Err(VideosonError::InvalidData(
                "mkv: no supported video track found",
            ));
        }

        Ok(Self {
            mkv,
            tracks,
            track_map,
        })
    }
}

#[cfg(feature = "std")]
impl Demuxer for MkvDemuxer {
    fn tracks(&self) -> &[Track] {
        self.tracks.as_slice()
    }

    fn next_packet(&mut self) -> videoson_core::Result<Option<Packet>> {
        loop {
            let mut frame = Frame::default();
            let has_frame = self
                .mkv
                .next_frame(&mut frame)
                .map_err(|_| VideosonError::InvalidData("mkv: next_frame failed"))?;
            if !has_frame {
                return Ok(None);
            }

            let Some(&track_id) = self.track_map.get(&frame.track) else {
                continue;
            };

            let mut pkt = Packet::new(track_id, frame.data);
            pkt.pts = Some(frame.timestamp as i64);
            pkt.dts = Some(frame.timestamp as i64);
            if let Some(dur) = frame.duration {
                pkt.duration = Some(dur as i64);
            }
            pkt.is_sync = frame.is_keyframe.unwrap_or(false);

            return Ok(Some(pkt));
        }
    }
}
