use std::collections::VecDeque;

use rav1d_safe::{Decoder, Frame, Planes};
use videoson_core::{
    CodecType, Packet, RegisterableVideoDecoder, Result, SupportedVideoCodec, VideoCodecParams,
    VideoDecoder, VideoDecoderOptions, VideoFrame, VideoOutputFormat, VideosonError,
    interleave_uv_nv12,
};

pub struct Rav1dSafeDecoder {
    params: VideoCodecParams,
    opts: VideoDecoderOptions,
    decoder: Decoder,
    queued: VecDeque<VideoFrame>,
    eos_sent: bool,
}

impl VideoDecoder for Rav1dSafeDecoder {
    fn try_new(params: &VideoCodecParams, opts: &VideoDecoderOptions) -> Result<Self> {
        if params.codec != CodecType::AV1 {
            return Err(VideosonError::InvalidData("not AV1"));
        }

        if matches!(opts.output_format, VideoOutputFormat::P010) {
            return Err(VideosonError::Unsupported(
                "P010 output is not supported for AV1",
            ));
        }

        let decoder =
            Decoder::new().map_err(|e| VideosonError::Message(format!("rav1d init: {e}")))?;
        Ok(Self {
            params: params.clone(),
            opts: *opts,
            decoder,
            queued: VecDeque::new(),
            eos_sent: false,
        })
    }

    fn codec_params(&self) -> &VideoCodecParams {
        &self.params
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        match self.decoder.decode(&packet.data) {
            Ok(Some(frame)) => {
                let vf = self.frame_to_video_frame(&frame, packet.pts)?;
                self.queued.push_back(vf);
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(rav1d_safe::Error::NeedMoreData) => Ok(()),
            Err(e) => Err(VideosonError::Message(format!("rav1d: {e}"))),
        }
    }

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>> {
        Ok(self.queued.pop_front())
    }

    fn send_eos(&mut self) -> Result<()> {
        if !self.eos_sent {
            self.eos_sent = true;
            let frames = self
                .decoder
                .flush()
                .map_err(|e| VideosonError::Message(format!("rav1d flush: {e}")))?;
            for frame in frames {
                let vf = self.frame_to_video_frame(&frame, None)?;
                self.queued.push_back(vf);
            }
        }
        Ok(())
    }

    fn reset(&mut self) {
        if let Ok(decoder) = Decoder::new() {
            self.decoder = decoder;
        }
        self.queued.clear();
        self.eos_sent = false;
    }

    fn output_format(&self) -> VideoOutputFormat {
        match self.opts.output_format {
            VideoOutputFormat::Nv12 => VideoOutputFormat::Nv12,
            VideoOutputFormat::Native | VideoOutputFormat::Yuv420 => VideoOutputFormat::Yuv420,
            VideoOutputFormat::P010 => VideoOutputFormat::Yuv420,
        }
    }
}

impl RegisterableVideoDecoder for Rav1dSafeDecoder {
    fn try_registry_new(
        params: &VideoCodecParams,
        opts: &VideoDecoderOptions,
    ) -> Result<std::boxed::Box<dyn VideoDecoder>> {
        Ok(std::boxed::Box::new(Self::try_new(params, opts)?))
    }

    fn supported_codecs() -> &'static [SupportedVideoCodec] {
        &[SupportedVideoCodec {
            codec_type: CodecType::AV1,
            short_name: "av1",
            long_name: "AV1 (rav1d-safe)",
        }]
    }
}

impl Rav1dSafeDecoder {
    fn wants_nv12(&self) -> bool {
        matches!(self.opts.output_format, VideoOutputFormat::Nv12)
    }

    fn frame_to_video_frame(&self, frame: &Frame, pts: Option<i64>) -> Result<VideoFrame> {
        let h = frame.height() as usize;
        let w = frame.width() as usize;

        match frame.planes() {
            Planes::Depth8(planes) => {
                let y_plane = planes.y();
                let u_plane = planes.u().unwrap_or_else(|| planes.y());
                let v_plane = planes.v().unwrap_or_else(|| planes.y());

                let u_stride = u_plane.row(0).len();
                let v_stride = v_plane.row(0).len();

                // Pack Y tightly to width.
                let mut y_data = Vec::with_capacity(w * h);
                for row in 0..h {
                    let row_bytes = y_plane.row(row);
                    let take = w.min(row_bytes.len());
                    y_data.extend_from_slice(&row_bytes[..take]);
                }

                let cw = (w + 1) / 2;
                let ch = (h + 1) / 2;

                if self.wants_nv12() {
                    let mut u_tmp = Vec::with_capacity(u_stride * ch);
                    let mut v_tmp = Vec::with_capacity(v_stride * ch);
                    for row in 0..ch {
                        u_tmp.extend_from_slice(u_plane.row(row));
                        v_tmp.extend_from_slice(v_plane.row(row));
                    }
                    let uv = interleave_uv_nv12(&u_tmp, u_stride, &v_tmp, v_stride, cw, ch);
                    Ok(VideoFrame::new_nv12_u8(
                        frame.width(),
                        frame.height(),
                        w,
                        cw * 2,
                        y_data,
                        uv,
                    )
                    .with_pts(pts))
                } else {
                    let mut u_data = Vec::with_capacity(cw * ch);
                    let mut v_data = Vec::with_capacity(cw * ch);
                    for row in 0..ch {
                        let u_row = u_plane.row(row);
                        let v_row = v_plane.row(row);
                        let take = cw.min(u_row.len()).min(v_row.len());
                        u_data.extend_from_slice(&u_row[..take]);
                        v_data.extend_from_slice(&v_row[..take]);
                    }
                    Ok(VideoFrame::new_yuv420_u8(
                        frame.width(),
                        frame.height(),
                        w,
                        cw,
                        cw,
                        y_data,
                        u_data,
                        v_data,
                    )
                    .with_pts(pts))
                }
            }
            Planes::Depth16(_) => Err(VideosonError::Unsupported("16-bit AV1 not supported")),
        }
    }
}
