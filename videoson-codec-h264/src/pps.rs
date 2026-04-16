use videoson_common::{read_se, read_ue, BitReader, BitstreamError, BitstreamResult};

#[derive(Debug, Clone)]
pub struct Pps {
    pub pps_id: u32,
    pub sps_id: u32,

    pub entropy_coding_mode_flag: bool,

    pub pic_init_qp_minus26: i32,
    pub chroma_qp_index_offset: i32,
}

pub fn parse_pps_rbsp(rbsp: &[u8]) -> BitstreamResult<Pps> {
    let mut br = BitReader::new(rbsp);

    let pps_id = read_ue(&mut br)?;
    let sps_id = read_ue(&mut br)?;

    let entropy_coding_mode_flag = br.read_bit()?;
    let _bottom_field_pic_order_in_frame_present_flag = br.read_bit()?;

    let num_slice_groups_minus1 = read_ue(&mut br)?;
    if num_slice_groups_minus1 != 0 {
        return Err(BitstreamError::Invalid("slice groups not supported in M0"));
    }

    let _num_ref_idx_l0_default_active_minus1 = read_ue(&mut br)?;
    let _num_ref_idx_l1_default_active_minus1 = read_ue(&mut br)?;

    let _weighted_pred_flag = br.read_bit()?;
    let _weighted_bipred_idc = br.read_bits_u32(2)?;

    let pic_init_qp_minus26 = read_se(&mut br)?;
    let _pic_init_qs_minus26 = read_se(&mut br)?;
    let chroma_qp_index_offset = read_se(&mut br)?;

    let _deblocking_filter_control_present_flag = br.read_bit()?;
    let _constrained_intra_pred_flag = br.read_bit()?;
    let _redundant_pic_cnt_present_flag = br.read_bit()?;

    Ok(Pps {
        pps_id,
        sps_id,
        entropy_coding_mode_flag,
        pic_init_qp_minus26,
        chroma_qp_index_offset,
    })
}
