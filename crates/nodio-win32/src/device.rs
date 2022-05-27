use std::mem::MaybeUninit;
use std::ptr::null;
use std::str::FromStr;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

use log::{error, trace, warn};
use notify_thread::JoinHandle;
use parking_lot::Mutex;
use widestring::U16Str;
use windows::core::{Interface, GUID, HSTRING, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio as windows_audio;
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::Win32::Media::Audio::{
    EDataFlow, IAudioSessionControl, IAudioSessionEnumerator, IAudioSessionManager2,
    IAudioSessionNotification, IMMDevice,
};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, STGM_READ, STGM_WRITE};
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Ole::{VT_BOOL, VT_LPWSTR};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PropVariantToBSTR, PROPERTYKEY};

use nodio_core::Uuid;

use crate::custom::{AudioSessionNotification, AudioSessionNotifications, DeviceState};
use crate::session::AudioSession;
use crate::{pwstr_to_string, Callback};

pub const DEVINTERFACE_AUDIO_RENDER: &str = "{e6327cad-dcec-4949-ae8a-991e976a79d2}";
pub const DEVINTERFACE_AUDIO_CAPTURE: &str = "{2eef81be-33fa-4800-9670-1cd474972c3f}";
pub const MMDEVAPI_TOKEN: &str = r#"\\?\SWD#MMDEVAPI#{0.0.0.00000000}."#;

pub struct AudioDevice {
    mmdevice: IMMDevice,

    audio_session_manager: IAudioSessionManager2,
    session_notifications: IAudioSessionNotification,
    endpoint_volume: Option<IAudioEndpointVolume>,
    meter: Option<IAudioMeterInformation>,
    name: String,

    id: Uuid,

    session_notification_callback: Arc<Mutex<Option<Callback<AudioSessionNotification>>>>,

    session_notification_thread: Option<JoinHandle<()>>,
}

impl Drop for AudioDevice {
    fn drop(&mut self) {
        trace!("dropping audio device {}", self.name);
        unsafe {
            self.audio_session_manager
                .UnregisterSessionNotification(self.session_notifications.clone())
                .ok();
        }
        if let Some(t) = self.session_notification_thread.take() {
            t.notify();
        }
        trace!("audio device dropped");
    }
}

impl AudioDevice {
    pub fn new(mmdevice: IMMDevice) -> windows::core::Result<Self> {
        unsafe {
            let audio_session_manager = mmdevice.activate::<IAudioSessionManager2>()?;

            let (session_notification_tx, session_notification_rx) = channel();
            let session_notifications = AudioSessionNotifications::new(session_notification_tx);
            audio_session_manager
                .RegisterSessionNotification(session_notifications.clone())
                .unwrap();

            let session_notification_callback: Arc<
                Mutex<Option<Callback<AudioSessionNotification>>>,
            > = Arc::new(Mutex::new(None));

            let session_notification_thread = {
                let session_notification_callback = session_notification_callback.clone();
                notify_thread::spawn(move |thread| loop {
                    match session_notification_rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(event) => {
                            trace!("Device session event: {:?}", event);

                            if let Some(cb) = session_notification_callback.lock().as_ref() {
                                cb(event);
                            }
                        }
                        _ if thread.notified() => {
                            trace!("Session notification thread ended");
                            return;
                        }
                        _ => {}
                    }
                })
            };

            let properties: IPropertyStore = mmdevice.OpenPropertyStore(STGM_READ)?;
            let name: PROPVARIANT = properties.GetValue(&PKEY_Device_FriendlyName)?;
            let name = U16Str::from_slice(PropVariantToBSTR(&name)?.as_wide()).to_string_lossy();

            let id = mmdevice.GetId().map(|id| {
                if id.is_null() {
                    Uuid::nil()
                } else {
                    pwstr_to_string(id)
                        .split_once("}.{")
                        .and_then(|(_, s)| s.split('}').next())
                        .and_then(|s| Uuid::from_str(s).ok())
                        .unwrap_or_else(Uuid::nil)
                }
            })?;

            let endpoint_volume: Option<IAudioEndpointVolume> = mmdevice
                .GetState()
                .ok()
                .filter(|state| *state == windows_audio::DEVICE_STATE_ACTIVE)
                .and_then(|_| mmdevice.activate().ok());

            let meter: Option<IAudioMeterInformation> = mmdevice
                .GetState()
                .ok()
                .filter(|state| *state == windows_audio::DEVICE_STATE_ACTIVE)
                .and_then(|_| mmdevice.activate().ok());

            Ok(Self {
                mmdevice,
                audio_session_manager,
                endpoint_volume,
                session_notifications,
                name,
                id,
                session_notification_callback,
                session_notification_thread: Some(session_notification_thread),
                meter,
            })
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn mmdevice(&self) -> &IMMDevice {
        &self.mmdevice
    }

    pub fn set_session_notification_callback<T>(&mut self, cb: T)
    where
        T: Fn(AudioSessionNotification) + Send + Sync + 'static,
    {
        let _ = self
            .session_notification_callback
            .lock()
            .insert(Box::new(cb));
    }

    pub fn set_listen(&self, target: Option<&AudioDevice>) -> windows::core::Result<()> {
        unsafe {
            let listen_state_prop_key = PROPERTYKEY {
                fmtid: GUID::from_u128(0x24DBB0FC_9311_4B3D_9CF0_18FF155639D4),
                pid: 1u32,
            };

            let listen_target_prop_key = PROPERTYKEY {
                fmtid: GUID::from_u128(0x24DBB0FC_9311_4B3D_9CF0_18FF155639D4),
                pid: 0u32,
            };

            let properties: IPropertyStore = self.mmdevice.OpenPropertyStore(STGM_WRITE)?;

            let mut listen_state_prop_value: PROPVARIANT = PROPVARIANT::default();
            (*(listen_state_prop_value.Anonymous.Anonymous)).vt = VT_BOOL.0 as _;
            (*(listen_state_prop_value.Anonymous.Anonymous))
                .Anonymous
                .boolVal = if target.is_some() { -1 } else { 0 };

            if let Some(target) = target {
                let mut target_device_id =
                    HSTRING::from(format!("{}.{{{}}}", "{0.0.0.00000000}", target.id()))
                        .as_wide()
                        .to_vec();
                let mut listen_target_prop_value: PROPVARIANT = PROPVARIANT::default();
                (*(listen_target_prop_value.Anonymous.Anonymous)).vt = VT_LPWSTR.0 as _;
                (*(listen_target_prop_value.Anonymous.Anonymous))
                    .Anonymous
                    .pwszVal = PWSTR(target_device_id.as_mut_ptr());

                properties.SetValue(&listen_target_prop_key, &listen_target_prop_value)?;
            }

            properties.SetValue(&listen_state_prop_key, &listen_state_prop_value)?;

            properties.Commit()?;
        }

        Ok(())
    }

    pub fn enumerate_sessions(&self) -> windows::core::Result<Vec<AudioSession>> {
        unsafe {
            let session_enumerator: IAudioSessionEnumerator = self
                .audio_session_manager
                .GetSessionEnumerator()
                .map_err(|err| {
                    error!("Failed to get session enumerator: {:?}", err);
                    err
                })?;

            let session_count = session_enumerator.GetCount().map_err(|err| {
                error!("Failed to get session count: {:?}", err);
                err
            })?;

            let mut sessions = Vec::with_capacity(session_count as usize);

            for i in 0..session_count {
                let control: IAudioSessionControl = match session_enumerator.GetSession(i) {
                    Ok(control) => control,
                    Err(err) => {
                        error!("Failed to get session control for session {}: {:?}", i, err);
                        continue;
                    }
                };

                let session = match AudioSession::new(control) {
                    Ok(session) => session,
                    Err(err) => {
                        error!("Failed to create session {}: {:?}", i, err);
                        continue;
                    }
                };

                if session.process_id() == 0 {
                    continue;
                }

                sessions.push(session);
            }

            Ok(sessions)
        }
    }

    pub fn is_active(&self) -> bool {
        self.state() == DeviceState::Active
    }

    pub fn state(&self) -> DeviceState {
        match unsafe { self.mmdevice.GetState() }.unwrap_or(windows_audio::DEVICE_STATE_DISABLED) {
            windows_audio::DEVICE_STATE_ACTIVE => DeviceState::Active,
            windows_audio::DEVICE_STATE_DISABLED => DeviceState::Disabled,
            windows_audio::DEVICE_STATE_NOTPRESENT => DeviceState::NotPresent,
            windows_audio::DEVICE_STATE_UNPLUGGED => DeviceState::Unplugged,
            _ => DeviceState::Disabled,
        }
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn set_master_volume(&self, volume: f32) {
        unsafe {
            if let Some(endpoint_volume) = self.endpoint_volume.as_ref() {
                if let Err(err) = endpoint_volume.SetMasterVolumeLevelScalar(volume, null()) {
                    warn!("Failed to get audio endpoint volume: {}", err);
                }
            }
        }
    }
    pub fn master_volume(&self) -> f32 {
        unsafe {
            self.endpoint_volume
                .as_ref()
                .and_then(|endpoint_volume| endpoint_volume.GetMasterVolumeLevelScalar().ok())
                .unwrap_or(0.0)
        }
    }

    pub fn peak_values(&self) -> windows::core::Result<(f32, f32)> {
        let meter = match self.meter.as_ref() {
            Some(meter) => meter,
            None => return Ok((0.0, 0.0)),
        };

        unsafe {
            let channel_count = usize::min(2, meter.GetMeteringChannelCount()? as usize);

            let mut values = [0.0; 2];
            meter.GetChannelsPeakValues(&mut values)?;

            if channel_count == 1 {
                Ok((values[0], values[0]))
            } else {
                Ok((values[0], values[1]))
            }
        }
    }

    pub fn mmdevice_id(&self, data_flow: EDataFlow) -> HSTRING {
        if self.id.is_nil() {
            HSTRING::new()
        } else {
            HSTRING::from(format!(
                r#"{}{{{}}}#{}"#,
                MMDEVAPI_TOKEN,
                self.id,
                match data_flow {
                    windows_audio::eCapture => DEVINTERFACE_AUDIO_CAPTURE,
                    _ => DEVINTERFACE_AUDIO_RENDER,
                }
            ))
        }
    }
}

pub trait MMDeviceExt {
    fn activate<T: Interface>(&self) -> windows::core::Result<T>;
}

impl MMDeviceExt for IMMDevice {
    fn activate<T: Interface>(&self) -> windows::core::Result<T> {
        unsafe {
            let mut result = MaybeUninit::<T>::uninit();

            self.Activate(&T::IID, CLSCTX_ALL, null(), result.as_mut_ptr() as _)?;

            Ok(result.assume_init())
        }
    }
}
