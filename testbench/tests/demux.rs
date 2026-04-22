use anyhow::Result;

use videoson_format_ivf::IvfDemuxer;

fn read_test_file(name: &str) -> Vec<u8> {
    std::fs::read(format!("tests/samples/{}", name)).expect(&format!("Failed to read {}", name))
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
