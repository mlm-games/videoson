// videoson-codec-h264/src/sps.rs
use videoson_common::{read_ue, BitReader, BitstreamError, BitstreamResult};

#[derive(Debug, Clone)]
pub struct Sps {
    pub profile_idc: u8,
    pub level_idc: u8,
    pub sps_id: u32,

    pub chroma_format_idc: u32,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,

    pub log2_max_frame_num_minus4: u32,
    pub pic_order_cnt_type: u32,
    pub log2_max_pic_order_cnt_lsb_minus4: u32,

    pub pic_width_in_mbs_minus1: u32,
    pub pic_height_in_map_units_minus1: u32,
    pub frame_mbs_only_flag: bool,

    pub frame_crop: Option<FrameCrop>,

    pub vui_full_range: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
pub struct FrameCrop {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

impl Sps {
    pub fn mbs_width(&self) -> u32 {
        self.pic_width_in_mbs_minus1 + 1
    }

    pub fn mbs_height(&self) -> u32 {
        self.pic_height_in_map_units_minus1 + 1
    }

    pub fn coded_width(&self) -> u32 {
        self.mbs_width() * 16
    }

    pub fn coded_height(&self) -> u32 {
        self.mbs_height() * 16
    }

    pub fn display_dimensions(&self) -> (u32, u32) {
        let cw = self.coded_width();
        let ch = self.coded_height();

        let Some(crop) = self.frame_crop else {
            return (cw, ch);
        };

        let frame_mbs_only = if self.frame_mbs_only_flag { 1u32 } else { 0u32 };
        let (sub_w, sub_h) = match self.chroma_format_idc {
            0 => (1u32, 1u32),
            1 => (2u32, 2u32),
            2 => (2u32, 1u32),
            3 => (1u32, 1u32),
            _ => (2u32, 2u32),
        };

        let crop_unit_x = sub_w;
        let crop_unit_y = sub_h * (2 - frame_mbs_only);

        let w = cw.saturating_sub((crop.left + crop.right) * crop_unit_x);
        let h = ch.saturating_sub((crop.top + crop.bottom) * crop_unit_y);
        (w, h)
    }

    pub fn frame_num_bits(&self) -> u32 {
        self.log2_max_frame_num_minus4 + 4
    }

    pub fn pic_order_cnt_lsb_bits(&self) -> u32 {
        self.log2_max_pic_order_cnt_lsb_minus4 + 4
    }
}

fn is_high_profile(profile_idc: u8) -> bool {
    matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    )
}

fn parse_vui(br: &mut BitReader<'_>) -> BitstreamResult<Option<bool>> {
    let aspect_ratio_info_present_flag = br.read_bit()?;
    if aspect_ratio_info_present_flag {
        let aspect_ratio_idc = br.read_bits_u32(8)?;
        if aspect_ratio_idc == 255 {
            let _sar_w = br.read_bits_u32(16)?;
            let _sar_h = br.read_bits_u32(16)?;
        }
    }

    let overscan_info_present_flag = br.read_bit()?;
    if overscan_info_present_flag {
        let _overscan_appropriate_flag = br.read_bit()?;
    }

    let video_signal_type_present_flag = br.read_bit()?;
    let mut full_range: Option<bool> = None;
    if video_signal_type_present_flag {
        let _video_format = br.read_bits_u32(3)?;
        let video_full_range_flag = br.read_bit()?;
        full_range = Some(video_full_range_flag);

        let colour_description_present_flag = br.read_bit()?;
        if colour_description_present_flag {
            let _colour_primaries = br.read_bits_u32(8)?;
            let _transfer_characteristics = br.read_bits_u32(8)?;
            let _matrix_coefficients = br.read_bits_u32(8)?;
        }
    }

    let chroma_loc_info_present_flag = br.read_bit()?;
    if chroma_loc_info_present_flag {
        let _chroma_sample_loc_type_top_field = read_ue(br)?;
        let _chroma_sample_loc_type_bottom_field = read_ue(br)?;
    }

    let timing_info_present_flag = br.read_bit()?;
    if timing_info_present_flag {
        let _num_units_in_tick = br.read_bits_u32(32)?;
        let _time_scale = br.read_bits_u32(32)?;
        let _fixed_frame_rate_flag = br.read_bit()?;
    }

    let nal_hrd_parameters_present_flag = br.read_bit()?;
    if nal_hrd_parameters_present_flag {
        return Err(BitstreamError::Invalid(
            "VUI HRD not supported in this minimal parser",
        ));
    }

    let vcl_hrd_parameters_present_flag = br.read_bit()?;
    if vcl_hrd_parameters_present_flag {
        return Err(BitstreamError::Invalid(
            "VUI HRD not supported in this minimal parser",
        ));
    }

    if nal_hrd_parameters_present_flag || vcl_hrd_parameters_present_flag {
        let _low_delay_hrd_flag = br.read_bit()?;
    }

    let _pic_struct_present_flag = br.read_bit()?;

    let bitstream_restriction_flag = br.read_bit()?;
    if bitstream_restriction_flag {
        let _motion_vectors_over_pic_boundaries_flag = br.read_bit()?;
        let _max_bytes_per_pic_denom = read_ue(br)?;
        let _max_bits_per_mb_denom = read_ue(br)?;
        let _log2_max_mv_length_horizontal = read_ue(br)?;
        let _log2_max_mv_length_vertical = read_ue(br)?;
        let _num_reorder_frames = read_ue(br)?;
        let _max_dec_frame_buffering = read_ue(br)?;
    }

    Ok(full_range)
}

pub fn parse_sps_rbsp(rbsp: &[u8]) -> BitstreamResult<Sps> {
    let mut br = BitReader::new(rbsp);

    let profile_idc = br.read_bits_u32(8)? as u8;
    let _constraint_and_reserved = br.read_bits_u32(8)? as u8;
    let level_idc = br.read_bits_u32(8)? as u8;

    let sps_id = read_ue(&mut br)?;

    let mut chroma_format_idc = 1u32;
    let mut bit_depth_luma = 8u8;
    let mut bit_depth_chroma = 8u8;

    if is_high_profile(profile_idc) {
        chroma_format_idc = read_ue(&mut br)?;
        if chroma_format_idc == 3 {
            let _separate_colour_plane_flag = br.read_bit()?;
        }
        let bit_depth_luma_minus8 = read_ue(&mut br)?;
        let bit_depth_chroma_minus8 = read_ue(&mut br)?;
        bit_depth_luma = (8 + bit_depth_luma_minus8) as u8;
        bit_depth_chroma = (8 + bit_depth_chroma_minus8) as u8;

        let _qpprime_y_zero_transform_bypass_flag = br.read_bit()?;
        let seq_scaling_matrix_present_flag = br.read_bit()?;
        if seq_scaling_matrix_present_flag {
            return Err(BitstreamError::Invalid(
                "SPS scaling matrices not supported in M0",
            ));
        }
    } else {
        chroma_format_idc = 1;
        bit_depth_luma = 8;
        bit_depth_chroma = 8;
    }

    let log2_max_frame_num_minus4 = read_ue(&mut br)?;
    let pic_order_cnt_type = read_ue(&mut br)?;

    let mut log2_max_pic_order_cnt_lsb_minus4 = 0u32;
    if pic_order_cnt_type == 0 {
        log2_max_pic_order_cnt_lsb_minus4 = read_ue(&mut br)?;
    } else {
        return Err(BitstreamError::Invalid(
            "pic_order_cnt_type != 0 not supported in M0",
        ));
    }

    let _max_num_ref_frames = read_ue(&mut br)?;
    let _gaps_in_frame_num_value_allowed_flag = br.read_bit()?;

    let pic_width_in_mbs_minus1 = read_ue(&mut br)?;
    let pic_height_in_map_units_minus1 = read_ue(&mut br)?;

    let frame_mbs_only_flag = br.read_bit()?;
    if !frame_mbs_only_flag {
        return Err(BitstreamError::Invalid(
            "interlaced not supported (frame_mbs_only_flag=0)",
        ));
    }

    let _direct_8x8_inference_flag = br.read_bit()?;

    let frame_cropping_flag = br.read_bit()?;
    let frame_crop = if frame_cropping_flag {
        Some(FrameCrop {
            left: read_ue(&mut br)?,
            right: read_ue(&mut br)?,
            top: read_ue(&mut br)?,
            bottom: read_ue(&mut br)?,
        })
    } else {
        None
    };

    let vui_parameters_present_flag = br.read_bit()?;
    let vui_full_range = if vui_parameters_present_flag {
        parse_vui(&mut br)?
    } else {
        None
    };

    Ok(Sps {
        profile_idc,
        level_idc,
        sps_id,
        chroma_format_idc,
        bit_depth_luma,
        bit_depth_chroma,
        log2_max_frame_num_minus4,
        pic_order_cnt_type,
        log2_max_pic_order_cnt_lsb_minus4,
        pic_width_in_mbs_minus1,
        pic_height_in_map_units_minus1,
        frame_mbs_only_flag,
        frame_crop,
        vui_full_range,
    })
}
