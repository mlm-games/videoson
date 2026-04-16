use anyhow::Result;

use videoson_core::Demuxer;
use videoson_format_ivf::IvfDemuxer;
use videoson_format_mkv::MkvDemuxer;
use videoson_format_mp4::Mp4Demuxer;

fn read_test_file(name: &str) -> Vec<u8> {
    std::fs::read(format!("tests/samples/{}", name)).expect(&format!("Failed to read {}", name))
}

#[test]
fn demux_mp4() -> Result<()> {
    let data = read_test_file("test_h264.mp4");
    let demux = Mp4Demuxer::new(&data)?;

    let tracks = demux.tracks();
    assert!(!tracks.is_empty(), "MP4 should have at least one track");

    let track = &tracks[0];
    assert!(track.params.coded_width > 0, "Track should have width");
    assert!(track.params.coded_height > 0, "Track should have height");

    println!(
        "MP4: track {}x{}",
        track.params.coded_width, track.params.coded_height
    );
    Ok(())
}

#[test]
fn demux_mkv() -> Result<()> {
    let data = read_test_file("test_h264.mkv");
    let mut demux = MkvDemuxer::new(data)?;

    let tracks = demux.tracks();
    assert!(!tracks.is_empty(), "MKV should have at least one track");

    let mut frame_count = 0;
    while let Some(pkt) = demux.next_packet()? {
        assert!(!pkt.data.is_empty(), "Packet should have data");
        frame_count += 1;
    }

    assert!(frame_count > 0, "MKV should have at least one frame");
    println!("MKV: {} frames demuxed", frame_count);
    Ok(())
}

#[test]
fn demux_ivf() -> Result<()> {
    let data = read_test_file("test_vp8.ivf");
    let mut demux = IvfDemuxer::new(data)?;

    let file_header = demux.file_header();
    assert!(file_header.width > 0, "IVF should have width");
    assert!(file_header.height > 0, "IVF should have height");

    let mut frame_count = 0;
    while let Some(pkt) = demux.next_packet()? {
        assert!(!pkt.data.is_empty(), "Packet should have data");
        frame_count += 1;
    }

    assert!(frame_count > 0, "IVF should have at least one frame");
    println!("IVF: {} frames demuxed", frame_count);
    Ok(())
}
