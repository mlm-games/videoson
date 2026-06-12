use std::collections::VecDeque;

use rav1d_safe::{Decoder, Frame, Planes};
use videoson_core::{
    CodecType, Packet, Result, VideoCodecParams, VideoDecoder,
    VideoDecoderOptions, VideoFrame, VideosonError,
    RegisterableVideoDecoder, SupportedVideoCodec,
};

pub struct Rav1dSafeDecoder {
    params: VideoCodecParams,
    decoder: Decoder,
    queued: VecDeque<VideoFrame>,
    eos_sent: bool,
}

impl VideoDecoder for Rav1dSafeDecoder {
    fn try_new(params: &VideoCodecParams, _opts: &VideoDecoderOptions) -> Result<Self> {
        if params.codec != CodecType::AV1 {
            return Err(VideosonError::InvalidData("not AV1"));
        }
        let decoder = Decoder::new()
            .map_err(|e| VideosonError::Message(format!("rav1d init: {e}")))?;
        Ok(Self {
            params: params.clone(),
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
                let vf = Self::frame_to_video_frame(&frame, packet.pts)?;
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
                let vf = Self::frame_to_video_frame(&frame, None)?;
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
    fn frame_to_video_frame(frame: &Frame, pts: Option<i64>) -> Result<VideoFrame> {
        let h = frame.height() as usize;

        match frame.planes() {
            Planes::Depth8(planes) => {
                let y_plane = planes.y();
                let u_plane = planes.u().unwrap_or_else(|| planes.y());
                let v_plane = planes.v().unwrap_or_else(|| planes.y());

                let y_stride = y_plane.row(0).len();
                let u_stride = u_plane.row(0).len();
                let v_stride = v_plane.row(0).len();

                let mut y_data = Vec::with_capacity(y_stride * h);
                let mut u_data = Vec::with_capacity(u_stride * (h / 2));
                let mut v_data = Vec::with_capacity(v_stride * (h / 2));

                for row in 0..h {
                    y_data.extend_from_slice(y_plane.row(row));
                }
                for row in 0..h / 2 {
                    u_data.extend_from_slice(u_plane.row(row));
                    v_data.extend_from_slice(v_plane.row(row));
                }

                Ok(VideoFrame::new_yuv420_u8(
                    frame.width(),
                    frame.height(),
                    y_stride, u_stride, v_stride,
                    y_data, u_data, v_data,
                )
                .with_pts(pts))
            }
            Planes::Depth16(_) => Err(VideosonError::Unsupported("16-bit AV1 not supported")),
        }
    }
}
