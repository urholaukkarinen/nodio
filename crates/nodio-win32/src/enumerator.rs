use crate::com::ensure_com_initialized;
use crate::custom::{DeviceNotification, DeviceNotifications};
use crate::device::AudioDevice;
use log::{trace, warn};
use std::sync::mpsc::channel;
use std::thread;
use windows::Win32::Media::Audio::{
    EDataFlow, ERole, IMMDeviceCollection, IMMDeviceEnumerator, IMMNotificationClient,
    MMDeviceEnumerator,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

pub struct AudioDeviceEnumerator {
    enumerator: IMMDeviceEnumerator,
    _device_notifications: IMMNotificationClient,
}

impl AudioDeviceEnumerator {
    pub fn create() -> windows::core::Result<Self> {
        ensure_com_initialized();

        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

            let (device_notification_tx, device_notification_rx) = channel::<DeviceNotification>();
            let device_notifications = DeviceNotifications::new(device_notification_tx);

            enumerator
                .RegisterEndpointNotificationCallback(device_notifications.clone())
                .expect("Failed to register endpoint notification callback");

            thread::spawn(move || {
                while let Ok(event) = device_notification_rx.recv() {
                    trace!("Device event: {:?}", event);

                    match event {
                        DeviceNotification::DefaultDeviceChanged { .. } => {}
                        DeviceNotification::DeviceAdded { .. } => {}
                        DeviceNotification::DeviceRemoved { .. } => {}
                        DeviceNotification::StateChanged { .. } => {}
                        DeviceNotification::PropertyChanged { .. } => {}
                    }
                }
            });

            Ok(Self {
                enumerator,
                _device_notifications: device_notifications,
            })
        }
    }

    pub fn _default_audio_endpoint(
        &self,
        data_flow: EDataFlow,
        role: ERole,
    ) -> windows::core::Result<AudioDevice> {
        unsafe {
            self.enumerator
                .GetDefaultAudioEndpoint(data_flow, role)
                .and_then(AudioDevice::new)
        }
    }

    pub fn enumerate_audio_endpoints(
        &self,
        data_flow: EDataFlow,
        state_mask: u32,
    ) -> windows::core::Result<Vec<AudioDevice>> {
        unsafe {
            let collection: IMMDeviceCollection =
                self.enumerator.EnumAudioEndpoints(data_flow, state_mask)?;

            let count = collection.GetCount()?;
            let mut endpoints = Vec::with_capacity(count as usize);

            for i in 0..count {
                match collection.Item(i).and_then(AudioDevice::new) {
                    Ok(device) => endpoints.push(device),
                    Err(err) => warn!("Could not get audio endpoint: {:?}", err),
                }
            }

            Ok(endpoints)
        }
    }
}
