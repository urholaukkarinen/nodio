use std::mem::size_of_val;
use std::ptr::{null, null_mut};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

use log::{trace, warn};
use notify_thread::JoinHandle;
use parking_lot::Mutex;
use widestring::U16Str;
use windows::core::{Interface, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HINSTANCE};
use windows::Win32::Media::Audio as windows_audio;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    AudioSessionState, IAudioSessionControl, IAudioSessionControl2, IAudioSessionEvents,
    ISimpleAudioVolume,
};
use windows::Win32::System::ProcessStatus::{
    K32EnumProcessModulesEx, K32GetModuleBaseNameW, K32GetModuleFileNameExW, LIST_MODULES_ALL,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};
use windows::Win32::UI::Shell::SHLoadIndirectString;

use nodio_core::{Node, NodeKind, Uuid};

use crate::custom::{AudioSessionEvent, AudioSessionEvents, SessionState};
use crate::pwstr_to_string;
use crate::Callback;

#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub enum AudioSessionKind {
    Application,
    Other,
}

#[derive(Clone)]
pub struct AudioSession {
    id: Uuid,
    process_id: u32,
    display_name: String,
    filename: String,
    kind: AudioSessionKind,
    control: IAudioSessionControl,
    simple_audio_volume: ISimpleAudioVolume,
    meter: IAudioMeterInformation,
    events: IAudioSessionEvents,
    event_thread_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    event_callback: Arc<Mutex<Option<Callback<AudioSessionEvent>>>>,
}

impl Drop for AudioSession {
    fn drop(&mut self) {
        trace!("dropping audio session {}", self.display_name);
        unsafe {
            self.control
                .UnregisterAudioSessionNotification(self.events.clone())
                .ok();
        }
        if let Some(t) = self.event_thread_handle.lock().take() {
            t.notify();
        }
        trace!("audio session dropped");
    }
}

unsafe impl Send for AudioSession {}
unsafe impl Sync for AudioSession {}

impl AudioSession {
    pub fn new(control: IAudioSessionControl) -> windows::core::Result<Self> {
        let control2: IAudioSessionControl2 = control.cast()?;
        let simple_audio_volume: ISimpleAudioVolume = control.cast()?;
        let meter: IAudioMeterInformation = control.cast()?;

        let process_id = unsafe { control2.GetProcessId()? };
        let display_name_pwstr: PWSTR = unsafe { control.GetDisplayName()? };
        let mut display_name: String = pwstr_to_string(display_name_pwstr);

        if display_name.starts_with('@') {
            let mut text = [0; 512];
            unsafe {
                SHLoadIndirectString(
                    PCWSTR(display_name_pwstr.0 as *const u16),
                    &mut text,
                    null_mut(),
                )?
            };

            let len = text.iter().take_while(|&&c| c != 0).count();

            display_name = String::from_utf16_lossy(&text[..len]);
        }

        if display_name.is_empty() {
            display_name = get_process_name(process_id)?;
        }

        let mut filename = String::new();
        if process_id != 0 {
            let handle = unsafe {
                OpenProcess(
                    PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
                    BOOL::from(false),
                    process_id,
                )
            }?;

            let mut module_filename = [0; 2048];
            let nread = unsafe {
                K32GetModuleFileNameExW(handle, HINSTANCE(0), module_filename.as_mut_slice())
            } as usize;

            filename = String::from_utf16_lossy(&module_filename[..nread]);
            unsafe { CloseHandle(handle) };
        }

        let kind = if filename.is_empty() {
            AudioSessionKind::Other
        } else {
            AudioSessionKind::Application
        };

        let (event_tx, event_rx) = channel();

        let events = AudioSessionEvents::create(event_tx);
        let session_event_callback: Arc<Mutex<Option<Callback<AudioSessionEvent>>>> =
            Arc::new(Mutex::new(None));
        let session_event_thread = {
            let session_event_callback = session_event_callback.clone();

            notify_thread::spawn(move |thread| loop {
                match event_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(event) => {
                        trace!("Session event: {:?}", event);

                        if let Some(cb) = session_event_callback.lock().as_ref() {
                            cb(event);
                        }
                    }

                    _ if thread.notified() => {
                        trace!("Session event thread ended");
                        return;
                    }
                    _ => {}
                }
            })
        };

        unsafe {
            control
                .RegisterAudioSessionNotification(events.clone())
                .unwrap();
        };

        Ok(Self {
            id: Uuid::new_v4(),
            process_id,
            display_name,
            filename,
            kind,
            control,
            simple_audio_volume,
            meter,
            events,
            event_thread_handle: Arc::new(Mutex::new(Some(session_event_thread))),
            event_callback: session_event_callback,
        })
    }

    pub fn set_event_callback<T>(&mut self, cb: T)
    where
        T: Fn(AudioSessionEvent) + Send + Sync + 'static,
    {
        let _ = self.event_callback.lock().insert(Box::new(cb));
    }

    pub fn is_active(&self) -> bool {
        let state: AudioSessionState = unsafe { self.control.GetState() }.unwrap();

        state == windows_audio::AudioSessionStateActive
    }

    pub fn set_master_volume(&self, volume: f32) {
        unsafe {
            if let Err(err) = self.simple_audio_volume.SetMasterVolume(volume, null()) {
                warn!(
                    "Failed to set volume for session {}: {:?}",
                    self.process_id, err
                );
            }
        }
    }

    pub fn master_volume(&self) -> f32 {
        unsafe { self.simple_audio_volume.GetMasterVolume().unwrap_or(0.0) as f32 }
    }

    pub fn _state(&self) -> SessionState {
        match unsafe { self.control.GetState() }.unwrap_or(windows_audio::AudioSessionStateExpired)
        {
            windows_audio::AudioSessionStateActive => SessionState::Active,
            windows_audio::AudioSessionStateInactive => SessionState::Inactive,
            windows_audio::AudioSessionStateExpired => SessionState::Expired,
            _ => SessionState::Expired,
        }
    }

    pub fn _muted(&self) -> bool {
        unsafe {
            self.simple_audio_volume
                .GetMute()
                .unwrap_or_default()
                .into()
        }
    }

    pub fn peak_values(&self) -> windows::core::Result<(f32, f32)> {
        unsafe {
            let channel_count = usize::min(2, self.meter.GetMeteringChannelCount()? as usize);

            let mut values = [0.0; 2];
            self.meter.GetChannelsPeakValues(values.as_mut_slice())?;

            if channel_count == 1 {
                Ok((values[0], values[0]))
            } else {
                Ok((values[0], values[1]))
            }
        }
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn kind(&self) -> AudioSessionKind {
        self.kind
    }
}

pub fn session_node_match(node: &Node, session: &AudioSession) -> bool {
    node.process_id == Some(session.process_id)
        || (node.kind == NodeKind::Application
            && node.display_name == session.display_name
            && node.filename == session.filename
            && session.kind == AudioSessionKind::Application)
        || (node.kind == NodeKind::InputDevice
            && node.display_name == session.display_name
            && session.kind == AudioSessionKind::Other)
}

fn get_process_name(pid: u32) -> windows::core::Result<String> {
    unsafe {
        let proc = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid)?;

        let mut hmodule = HINSTANCE::default();
        let mut bytes_needed = 0;
        if K32EnumProcessModulesEx(
            proc,
            &mut hmodule,
            size_of_val(&hmodule) as _,
            &mut bytes_needed,
            LIST_MODULES_ALL,
        )
        .as_bool()
        {
            let mut name: [u16; 256] = [0; 256];
            let len = K32GetModuleBaseNameW(proc, hmodule, name.as_mut_slice());

            Ok(U16Str::from_ptr(name.as_ptr(), len as _).to_string_lossy())
        } else {
            Ok(String::default())
        }
    }
}
