use crate::device::MMDeviceExt;
use log::warn;
use std::ptr::null;
use windows::Win32::Media::Audio::{
    IAudioClient, IAudioRenderClient, IMMDevice, AUDCLNT_SHAREMODE_SHARED, WAVEFORMATEX,
    WAVEFORMATEXTENSIBLE,
};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;

pub struct RenderClient {
    audio_client: IAudioClient,
    render_client: IAudioRenderClient,
    wave_format: WAVEFORMATEXTENSIBLE,
}

impl Drop for RenderClient {
    fn drop(&mut self) {
        if let Err(err) = unsafe { self.audio_client.Stop() } {
            warn!("Could not stop render client: {}", err);
        }
    }
}

impl RenderClient {
    pub fn new(device: &IMMDevice) -> windows::core::Result<Self> {
        unsafe {
            let audio_client = device.activate::<IAudioClient>()?;
            let pwfx: *mut WAVEFORMATEX = audio_client.GetMixFormat()?;
            audio_client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 0, 0, pwfx, null())?;
            let render_client = audio_client.GetService::<IAudioRenderClient>()?;

            let mut wave_format: WAVEFORMATEXTENSIBLE = std::mem::zeroed();

            if (*pwfx).wFormatTag == WAVE_FORMAT_EXTENSIBLE as _ {
                wave_format = *(pwfx as *mut WAVEFORMATEXTENSIBLE)
            } else {
                wave_format.Format = *pwfx;
            }

            audio_client.Start()?;

            Ok(Self {
                audio_client,
                render_client,
                wave_format,
            })
        }
    }

    pub fn wave_format(&self) -> &WAVEFORMATEXTENSIBLE {
        &self.wave_format
    }

    pub fn render_frames(&self, data_in: *const u8, frames: u32) -> windows::core::Result<()> {
        unsafe {
            let padding = self.audio_client.GetCurrentPadding()?;
            let frames = frames - padding;

            let data_out = self.render_client.GetBuffer(frames)?;

            let data_len = frames * self.wave_format.Format.nBlockAlign as u32;

            std::ptr::copy(data_in, data_out, data_len as usize);

            self.render_client.ReleaseBuffer(frames, 0)?;
        }

        Ok(())
    }
}
