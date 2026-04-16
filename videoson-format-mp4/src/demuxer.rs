extern crate alloc;

use alloc::{vec, vec::Vec};

use videoson_core::{CodecType, NalFormat, Packet, VideoCodecParams, VideosonError};

use videoson_common::parse_avcc_extradata;

use crate::atom::{find_child, iter_children};

#[derive(Debug, Clone)]
pub struct Mp4Track {
    pub id: u32,
    pub params: VideoCodecParams,
    pub timescale: u32,
    pub sample_count: usize,
}

#[derive(Debug, Clone)]
struct SampleIndex {
    offsets: Vec<u64>,
    sizes: Vec<u32>,
    dts: Vec<i64>,
    pts: Vec<i64>,
    dur: Vec<i64>,
    sync_samples: Vec<u32>, // 1-based sample numbers that are sync (keyframes)
}

pub struct Mp4Demuxer<'a> {
    data: &'a [u8],
    tracks: Vec<Mp4Track>,
    indices: Vec<SampleIndex>,
    curs: Vec<usize>, // current sample index for each track
}

fn be_u16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn be_u64(b: &[u8]) -> u64 {
    u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

fn parse_tkhd_track_id(data: &[u8], ps: usize, pe: usize) -> Result<Option<u32>, VideosonError> {
    if pe <= ps || pe - ps < 24 {
        return Ok(None);
    }
    let version = data[ps];
    let off = match version {
        0 => 12,
        1 => 20,
        _ => return Ok(None),
    };
    if pe - ps < off + 4 {
        return Ok(None);
    }
    Ok(Some(be_u32(&data[ps + off..ps + off + 4])))
}

fn read_fullbox_version(flags: &[u8]) -> u8 {
    flags[0]
}

fn parse_mdhd_timescale(data: &[u8], ps: usize, pe: usize) -> Result<u32, VideosonError> {
    if pe - ps < 4 {
        return Err(VideosonError::InvalidData("mp4: mdhd too short"));
    }
    let v = read_fullbox_version(&data[ps..ps + 4]);
    let mut p = ps + 4;
    match v {
        0 => {
            if pe - p < 4 + 4 + 4 {
                return Err(VideosonError::InvalidData("mp4: mdhd v0 too short"));
            }
            p += 4; // creation
            p += 4; // modification
            let timescale = be_u32(&data[p..p + 4]);
            Ok(timescale)
        }
        1 => {
            if pe - p < 8 + 8 + 4 {
                return Err(VideosonError::InvalidData("mp4: mdhd v1 too short"));
            }
            p += 8;
            p += 8;
            let timescale = be_u32(&data[p..p + 4]);
            Ok(timescale)
        }
        _ => Err(VideosonError::Unsupported(
            "mp4: mdhd version not supported",
        )),
    }
}

fn parse_hdlr_type(data: &[u8], ps: usize, pe: usize) -> Result<[u8; 4], VideosonError> {
    if pe - ps < 4 + 4 + 4 {
        return Err(VideosonError::InvalidData("mp4: hdlr too short"));
    }
    let mut p = ps + 4;
    p += 4;
    Ok(data[p..p + 4].try_into().unwrap())
}

fn parse_stsz(data: &[u8], ps: usize, pe: usize) -> Result<Vec<u32>, VideosonError> {
    if pe - ps < 12 {
        return Err(VideosonError::InvalidData("mp4: stsz too short"));
    }
    let mut p = ps + 4;
    let sample_size = be_u32(&data[p..p + 4]);
    p += 4;
    let sample_count = be_u32(&data[p..p + 4]) as usize;
    p += 4;

    if sample_size != 0 {
        return Ok(vec![sample_size; sample_count]);
    }

    if pe - p < sample_count * 4 {
        return Err(VideosonError::InvalidData("mp4: stsz table too short"));
    }
    let mut sizes = Vec::with_capacity(sample_count);
    for _ in 0..sample_count {
        sizes.push(be_u32(&data[p..p + 4]));
        p += 4;
    }
    Ok(sizes)
}

fn parse_stss(data: &[u8], ps: usize, pe: usize) -> Result<Vec<u32>, VideosonError> {
    if pe - ps < 8 {
        return Err(VideosonError::InvalidData("mp4: stss too short"));
    }
    let mut p = ps + 4; // skip fullbox version/flags
    let count = be_u32(&data[p..p + 4]) as usize;
    p += 4;

    if count == 0 {
        return Ok(Vec::new());
    }

    if pe - p < count * 4 {
        return Err(VideosonError::InvalidData("mp4: stss table too short"));
    }
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(be_u32(&data[p..p + 4]));
        p += 4;
    }
    Ok(entries)
}

fn parse_stco_u32(data: &[u8], ps: usize, pe: usize) -> Result<Vec<u64>, VideosonError> {
    let mut p = ps + 4;
    let count = be_u32(&data[p..p + 4]) as usize;
    p += 4;
    if pe - p < count * 4 {
        return Err(VideosonError::InvalidData("mp4: stco too short"));
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(be_u32(&data[p..p + 4]) as u64);
        p += 4;
    }
    Ok(out)
}

fn parse_co64_u64(data: &[u8], ps: usize, pe: usize) -> Result<Vec<u64>, VideosonError> {
    let mut p = ps + 4;
    let count = be_u32(&data[p..p + 4]) as usize;
    p += 4;
    if pe - p < count * 8 {
        return Err(VideosonError::InvalidData("mp4: co64 too short"));
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(be_u64(&data[p..p + 8]));
        p += 8;
    }
    Ok(out)
}

fn parse_stsc(data: &[u8], ps: usize, pe: usize) -> Result<Vec<(u32, u32, u32)>, VideosonError> {
    if pe - ps < 8 {
        return Err(VideosonError::InvalidData("mp4: stsc too short"));
    }
    let mut p = ps + 4;
    let count = be_u32(&data[p..p + 4]) as usize;
    p += 4;
    if pe - p < count * 12 {
        return Err(VideosonError::InvalidData("mp4: stsc table too short"));
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let first_chunk = be_u32(&data[p..p + 4]);
        p += 4;
        let samples_per_chunk = be_u32(&data[p..p + 4]);
        p += 4;
        let sample_desc_idx = be_u32(&data[p..p + 4]);
        p += 4;
        out.push((first_chunk, samples_per_chunk, sample_desc_idx));
    }
    Ok(out)
}

fn parse_stts(data: &[u8], ps: usize, pe: usize) -> Result<Vec<(u32, u32)>, VideosonError> {
    if pe - ps < 8 {
        return Err(VideosonError::InvalidData("mp4: stts too short"));
    }
    let mut p = ps + 4;
    let count = be_u32(&data[p..p + 4]) as usize;
    p += 4;
    if pe - p < count * 8 {
        return Err(VideosonError::InvalidData("mp4: stts table too short"));
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let sample_count = be_u32(&data[p..p + 4]);
        p += 4;
        let sample_delta = be_u32(&data[p..p + 4]);
        p += 4;
        out.push((sample_count, sample_delta));
    }
    Ok(out)
}

fn parse_ctts(data: &[u8], ps: usize, pe: usize) -> Result<Vec<i64>, VideosonError> {
    if pe - ps < 8 {
        return Err(VideosonError::InvalidData("mp4: ctts too short"));
    }
    let v = data[ps];
    let mut p = ps + 4;
    let count = be_u32(&data[p..p + 4]) as usize;
    p += 4;

    let mut out: Vec<i64> = Vec::new();
    for _ in 0..count {
        if pe - p < 8 {
            return Err(VideosonError::InvalidData("mp4: ctts entry too short"));
        }
        let sc = be_u32(&data[p..p + 4]) as usize;
        p += 4;
        let off_u = be_u32(&data[p..p + 4]);
        p += 4;
        let off = if v == 1 {
            (off_u as i32) as i64
        } else {
            off_u as i64
        };
        out.extend(core::iter::repeat_n(off, sc));
    }
    Ok(out)
}

fn expand_stts(stts: &[(u32, u32)], sample_count: usize) -> (Vec<i64>, Vec<i64>) {
    let mut dts = Vec::with_capacity(sample_count);
    let mut dur = Vec::with_capacity(sample_count);

    let mut cur: i64 = 0;
    for &(cnt, delta) in stts {
        for _ in 0..(cnt as usize) {
            dts.push(cur);
            dur.push(delta as i64);
            cur += delta as i64;
            if dts.len() == sample_count {
                return (dts, dur);
            }
        }
    }
    (dts, dur)
}

fn build_sample_offsets(
    chunk_offsets: &[u64],
    stsc: &[(u32, u32, u32)],
    sizes: &[u32],
) -> Result<Vec<u64>, VideosonError> {
    let chunk_count = chunk_offsets.len();
    let mut spc = vec![0u32; chunk_count];

    for (i, &(first_chunk, samples_per_chunk, _sd)) in stsc.iter().enumerate() {
        let start = (first_chunk as usize).saturating_sub(1);
        let end = if i + 1 < stsc.len() {
            (stsc[i + 1].0 as usize).saturating_sub(1)
        } else {
            chunk_count
        };
        for c in start..end.min(chunk_count) {
            spc[c] = samples_per_chunk;
        }
    }

    let mut offsets = Vec::with_capacity(sizes.len());
    let mut sample_i = 0usize;

    for (chunk_i, &co) in chunk_offsets.iter().enumerate() {
        let n = spc[chunk_i] as usize;
        let mut off = co;
        for _ in 0..n {
            if sample_i >= sizes.len() {
                return Ok(offsets);
            }
            offsets.push(off);
            off += sizes[sample_i] as u64;
            sample_i += 1;
        }
    }

    Ok(offsets)
}

const VISUAL_SAMPLE_ENTRY_HEADER_LEN: usize = 78;

fn parse_visual_sample_entry_dimensions(
    data: &[u8],
    entry_ps: usize,
    entry_pe: usize,
) -> Option<(u32, u32)> {
    if entry_pe < entry_ps || entry_pe - entry_ps < 28 {
        return None;
    }
    let w = be_u16(&data[entry_ps + 24..entry_ps + 26]) as u32;
    let h = be_u16(&data[entry_ps + 26..entry_ps + 28]) as u32;
    if w == 0 || h == 0 {
        None
    } else {
        Some((w, h))
    }
}

fn parse_stsd_and_codec(
    data: &[u8],
    ps: usize,
    pe: usize,
) -> Result<(VideoCodecParams, u8), VideosonError> {
    if pe - ps < 8 {
        return Err(VideosonError::InvalidData("mp4: stsd too short"));
    }
    let mut p = ps + 4;
    let entry_count = be_u32(&data[p..p + 4]) as usize;
    p += 4;
    if entry_count == 0 {
        return Err(VideosonError::InvalidData("mp4: stsd has 0 entries"));
    }

    if pe - p < 8 {
        return Err(VideosonError::InvalidData("mp4: stsd entry too short"));
    }

    let entry_size = be_u32(&data[p..p + 4]) as usize;
    let entry_type: [u8; 4] = data[p + 4..p + 8].try_into().unwrap();

    if entry_size < 16 || p + entry_size > pe {
        return Err(VideosonError::InvalidData("mp4: bad stsd entry size"));
    }

    let entry_ps = p + 8;
    let entry_pe = p + entry_size;

    let codec = match &entry_type {
        b"avc1" | b"avc3" => CodecType::H264,
        b"av01" => CodecType::AV1,
        _ => {
            return Err(VideosonError::Unsupported(
                "mp4: unsupported video sample entry",
            ))
        }
    };

    let mut params = VideoCodecParams::new(codec);
    if let Some((w, h)) = parse_visual_sample_entry_dimensions(data, entry_ps, entry_pe) {
        params.coded_width = w;
        params.coded_height = h;
    }

    let mut nal_len_size: u8 = 4;

    // Child boxes start after the VisualSampleEntry header.
    let child_ps = entry_ps
        .saturating_add(VISUAL_SAMPLE_ENTRY_HEADER_LEN)
        .min(entry_pe);
    for child in iter_children(data, child_ps, entry_pe) {
        let (h, cps, cpe) = child?;
        if codec == CodecType::H264 && &h.typ == b"avcC" {
            params.extradata = data[(cps - 8)..cpe].to_vec();
            let cfg = parse_avcc_extradata(&data[cps..cpe])
                .map_err(|_| VideosonError::InvalidData("mp4: bad avcC"))?;
            nal_len_size = cfg.nal_len_size;
            params.nal_format = Some(NalFormat::Avcc { nal_len_size });
        }
        if codec == CodecType::AV1 && &h.typ == b"av1C" {
            params.extradata = data[(cps - 8)..cpe].to_vec();
        }
    }

    Ok((params, nal_len_size))
}

impl<'a> Mp4Demuxer<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self, VideosonError> {
        let (moov_ps, moov_pe) = {
            let mut found = None;
            for child in iter_children(data, 0, data.len()) {
                let (h, ps, pe) = child?;
                if &h.typ == b"moov" {
                    found = Some((ps, pe));
                    break;
                }
            }
            found.ok_or(VideosonError::InvalidData("mp4: missing moov"))?
        };

        let mut all_tracks: Vec<(Mp4Track, SampleIndex)> = Vec::new();

        for trak in iter_children(data, moov_ps, moov_pe) {
            let (h, trak_ps, trak_pe) = trak?;
            if &h.typ != b"trak" {
                continue;
            }

            let Some((mdia_ps, mdia_pe)) = find_child(data, trak_ps, trak_pe, b"mdia")? else {
                continue;
            };

            let Some((hdlr_ps, hdlr_pe)) = find_child(data, mdia_ps, mdia_pe, b"hdlr")? else {
                continue;
            };
            let handler = parse_hdlr_type(data, hdlr_ps, hdlr_pe)?;
            if &handler != b"vide" {
                continue;
            }

            // Track ID from tkhd
            let mut track_id: u32 = 0;
            if let Some((tkhd_ps, tkhd_pe)) = find_child(data, trak_ps, trak_pe, b"tkhd")? {
                if let Some(tid) = parse_tkhd_track_id(data, tkhd_ps, tkhd_pe)? {
                    track_id = tid;
                }
            }

            let Some((mdhd_ps, mdhd_pe)) = find_child(data, mdia_ps, mdia_pe, b"mdhd")? else {
                return Err(VideosonError::InvalidData("mp4: vide track missing mdhd"));
            };
            let timescale = parse_mdhd_timescale(data, mdhd_ps, mdhd_pe)?;

            let Some((minf_ps, minf_pe)) = find_child(data, mdia_ps, mdia_pe, b"minf")? else {
                return Err(VideosonError::InvalidData("mp4: vide track missing minf"));
            };
            let Some((stbl_ps, stbl_pe)) = find_child(data, minf_ps, minf_pe, b"stbl")? else {
                return Err(VideosonError::InvalidData("mp4: vide track missing stbl"));
            };

            let (stsd_ps, stsd_pe) = find_child(data, stbl_ps, stbl_pe, b"stsd")?
                .ok_or(VideosonError::InvalidData("mp4: missing stsd"))?;
            let (mut params, _nal_len) = parse_stsd_and_codec(data, stsd_ps, stsd_pe)?;

            let (stts_ps, stts_pe) = find_child(data, stbl_ps, stbl_pe, b"stts")?
                .ok_or(VideosonError::InvalidData("mp4: missing stts"))?;
            let stts = parse_stts(data, stts_ps, stts_pe)?;

            let (stsc_ps, stsc_pe) = find_child(data, stbl_ps, stbl_pe, b"stsc")?
                .ok_or(VideosonError::InvalidData("mp4: missing stsc"))?;
            let stsc = parse_stsc(data, stsc_ps, stsc_pe)?;

            let (stsz_ps, stsz_pe) = find_child(data, stbl_ps, stbl_pe, b"stsz")?
                .ok_or(VideosonError::InvalidData("mp4: missing stsz"))?;
            let sizes = parse_stsz(data, stsz_ps, stsz_pe)?;

            let chunk_offsets = if let Some((stco_ps, stco_pe)) =
                find_child(data, stbl_ps, stbl_pe, b"stco")?
            {
                parse_stco_u32(data, stco_ps, stco_pe)?
            } else if let Some((co64_ps, co64_pe)) = find_child(data, stbl_ps, stbl_pe, b"co64")? {
                parse_co64_u64(data, co64_ps, co64_pe)?
            } else {
                return Err(VideosonError::InvalidData("mp4: missing stco/co64"));
            };

            let offsets = build_sample_offsets(&chunk_offsets, &stsc, &sizes)?;

            let sample_count = sizes.len();
            let (dts, dur) = expand_stts(&stts, sample_count);

            let ctts =
                if let Some((ctts_ps, ctts_pe)) = find_child(data, stbl_ps, stbl_pe, b"ctts")? {
                    Some(parse_ctts(data, ctts_ps, ctts_pe)?)
                } else {
                    None
                };

            let mut pts = Vec::with_capacity(sample_count);
            for i in 0..sample_count {
                let off = ctts.as_ref().and_then(|v| v.get(i)).copied().unwrap_or(0);
                pts.push(dts[i] + off);
            }

            // Parse stss (sync sample table)
            // If absent, all samples are sync (keyframes)
            let sync_entries =
                if let Some((stss_ps, stss_pe)) = find_child(data, stbl_ps, stbl_pe, b"stss")? {
                    parse_stss(data, stss_ps, stss_pe)?
                } else {
                    Vec::new() // empty means all samples are sync
                };

            let track = Mp4Track {
                id: if track_id != 0 {
                    track_id
                } else {
                    all_tracks.len() as u32
                },
                params,
                timescale,
                sample_count,
            };
            let idx = SampleIndex {
                offsets,
                sizes,
                dts,
                pts,
                dur,
                sync_samples: sync_entries,
            };

            all_tracks.push((track, idx));
        }

        if all_tracks.is_empty() {
            return Err(VideosonError::InvalidData("mp4: no video track found"));
        }

        let tracks: Vec<_> = all_tracks.iter().map(|(t, _)| t.clone()).collect();
        let indices: Vec<_> = all_tracks.iter().map(|(_, i)| i.clone()).collect();
        let curs: Vec<usize> = vec![0; tracks.len()];

        Ok(Self {
            data,
            tracks,
            indices,
            curs,
        })
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn tracks(&self) -> &[Mp4Track] {
        &self.tracks
    }

    pub fn next_packet(&mut self) -> Result<Option<Packet>, VideosonError> {
        // Find the track with the earliest next sample (by DTS)
        let mut best_track = None;
        let mut best_dts: i64 = i64::MAX;

        for (track_idx, cur) in self.curs.iter().enumerate() {
            if *cur >= self.tracks[track_idx].sample_count {
                continue;
            }
            let dts = self.indices[track_idx].dts[*cur];
            if dts < best_dts {
                best_dts = dts;
                best_track = Some(track_idx);
            }
        }

        let track_idx = best_track.ok_or(VideosonError::InvalidData(
            "mp4: no tracks have more samples",
        ))?;

        let i = self.curs[track_idx];
        self.curs[track_idx] += 1;

        let off = self.indices[track_idx].offsets[i] as usize;
        let sz = self.indices[track_idx].sizes[i] as usize;

        if off + sz > self.data.len() {
            return Err(VideosonError::InvalidData("mp4: sample out of range"));
        }

        let sample_num = (i + 1) as u32;
        let is_sync = if self.indices[track_idx].sync_samples.is_empty() {
            true
        } else {
            self.indices[track_idx].sync_samples.contains(&sample_num)
        };

        let mut pkt = Packet::new(self.tracks[track_idx].id, self.data[off..off + sz].to_vec());
        pkt.dts = Some(self.indices[track_idx].dts[i]);
        pkt.pts = Some(self.indices[track_idx].pts[i]);
        pkt.duration = Some(self.indices[track_idx].dur[i]);
        pkt.is_sync = is_sync;
        Ok(Some(pkt))
    }
}
