#![allow(non_snake_case)]

use std::ffi::c_void;
use std::fmt::Debug;
use std::mem;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ptr::null_mut;
use std::sync::mpsc::Sender;

use log::warn;
use widestring::U16CStr;
use windows::core::{IUnknown, IUnknownVtbl, PCWSTR};
use windows::Win32::Foundation::{BOOL, E_NOINTERFACE, S_OK};
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eConsole, eMultimedia, eRender, AudioSessionDisconnectReason,
    AudioSessionState, AudioSessionStateActive, AudioSessionStateExpired,
    AudioSessionStateInactive, DisconnectReasonDeviceRemoval,
    DisconnectReasonExclusiveModeOverride, DisconnectReasonFormatChanged,
    DisconnectReasonServerShutdown, DisconnectReasonSessionDisconnected,
    DisconnectReasonSessionLogoff, IAudioSessionControl, IAudioSessionControl2,
    IAudioSessionEvents, IAudioSessionEvents_Vtbl, IAudioSessionNotification,
    IAudioSessionNotification_Vtbl, IMMNotificationClient, IMMNotificationClient_Vtbl,
    AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED,
    DEVICE_STATE_NOTPRESENT, DEVICE_STATE_UNPLUGGED,
};
use windows::Win32::System::Registry::{
    GetRegistryValueWithFallbackW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
};
use windows::Win32::System::WinRT::RoGetActivationFactory;
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
use windows::{
    core::{
        IInspectable, IInspectableVtbl, Interface, IntoParam, Param, RawPtr, Result, GUID, HRESULT,
        HSTRING,
    },
    Win32::Media::Audio::{EDataFlow, ERole},
};

fn os_version() -> u32 {
    let mut os_version: [u16; 512] = [0; 512];

    let status = unsafe {
        GetRegistryValueWithFallbackW(
            HKEY_LOCAL_MACHINE,
            "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
            HKEY_LOCAL_MACHINE,
            "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
            "CurrentBuild",
            RRF_RT_REG_SZ.0,
            null_mut(),
            &mut os_version as *mut _ as _,
            512,
            null_mut(),
        )
    }
    .0;

    if status != 0 {
        panic!("Failed to get os version: WIN32_ERROR({})", status);
    }

    HSTRING::from_wide(
        &os_version
            .iter()
            .take_while(|&&c| c != 0)
            .copied()
            .collect::<Vec<_>>(),
    )
    .to_string_lossy()
    .parse::<u32>()
    .expect("Failed to parse os version")
}

pub fn create_audio_policy_config() -> Box<dyn AudioPolicyConfig> {
    const LATEST_GUID: u128 = 0xab3d4648_e242_459f_b02f_541c70306324;
    const LEGACY_GUID: u128 = 0x2a59116d_6c4f_45e0_a74f_707e3fef9258;

    let name = HSTRING::from("Windows.Media.Internal.AudioPolicyConfig");

    unsafe {
        if os_version() >= 21390 {
            Box::new(RoGetActivationFactory::<_, IAudioPolicyConfig<LATEST_GUID>>(name).unwrap())
        } else {
            Box::new(RoGetActivationFactory::<_, IAudioPolicyConfig<LEGACY_GUID>>(name).unwrap())
        }
    }
}

pub trait AudioPolicyConfig {
    unsafe fn persistent_default_audio_endpoint(
        &self,
        process_id: u32,
        data_flow: EDataFlow,
        role: ERole,
    ) -> Result<HSTRING>;

    unsafe fn set_persistent_default_audio_endpoint(
        &self,
        process_id: u32,
        data_flow: EDataFlow,
        role: ERole,
        device_id: HSTRING,
    ) -> Result<()>;

    unsafe fn clear_all_persisted_default_endpoints(&self) -> Result<()>;
}

impl<const T: u128> AudioPolicyConfig for IAudioPolicyConfig<T> {
    unsafe fn persistent_default_audio_endpoint(
        &self,
        process_id: u32,
        data_flow: EDataFlow,
        role: ERole,
    ) -> Result<HSTRING> {
        self.GetPersistedDefaultAudioEndpoint(process_id, data_flow, role)
    }

    unsafe fn set_persistent_default_audio_endpoint(
        &self,
        process_id: u32,
        data_flow: EDataFlow,
        role: ERole,
        device_id: HSTRING,
    ) -> Result<()> {
        self.SetPersistedDefaultAudioEndpoint(process_id, data_flow, role, device_id)
    }

    unsafe fn clear_all_persisted_default_endpoints(&self) -> Result<()> {
        self.ClearAllPersistedApplicationDefaultEndpoints()
    }
}

#[repr(transparent)]
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct IAudioPolicyConfig<const T: u128>(IInspectable);
impl<const T: u128> IAudioPolicyConfig<T> {
    pub unsafe fn GetPersistedDefaultAudioEndpoint(
        &self,
        process_id: u32,
        data_flow: EDataFlow,
        role: ERole,
    ) -> Result<HSTRING> {
        let mut result__ = MaybeUninit::<ManuallyDrop<HSTRING>>::zeroed();
        (Interface::vtable(self).GetPersistedDefaultAudioEndpoint)(
            mem::transmute_copy(self),
            process_id,
            data_flow,
            role,
            result__.as_mut_ptr(),
        )
        .from_abi::<HSTRING>(result__)
    }

    pub unsafe fn SetPersistedDefaultAudioEndpoint(
        &self,
        process_id: u32,
        data_flow: EDataFlow,
        role: ERole,
        device_id: HSTRING,
    ) -> Result<()> {
        (Interface::vtable(self).SetPersistedDefaultAudioEndpoint)(
            mem::transmute_copy(self),
            process_id,
            data_flow,
            role,
            device_id,
        )
        .ok()
    }

    pub unsafe fn ClearAllPersistedApplicationDefaultEndpoints(&self) -> Result<()> {
        (Interface::vtable(self).ClearAllPersistedApplicationDefaultEndpoints)(mem::transmute_copy(
            self,
        ))
        .ok()
    }
}
impl<const T: u128> From<IAudioPolicyConfig<T>> for IInspectable {
    fn from(value: IAudioPolicyConfig<T>) -> Self {
        unsafe { mem::transmute(value) }
    }
}

impl<const T: u128> From<&IAudioPolicyConfig<T>> for IInspectable {
    fn from(value: &IAudioPolicyConfig<T>) -> Self {
        From::from(Clone::clone(value))
    }
}

impl<'a, const T: u128> IntoParam<'a, IInspectable> for IAudioPolicyConfig<T> {
    fn into_param(self) -> Param<'a, IInspectable> {
        Param::Owned(unsafe { mem::transmute(self) })
    }
}

impl<'a, const T: u128> IntoParam<'a, IInspectable> for &IAudioPolicyConfig<T> {
    fn into_param(self) -> Param<'a, IInspectable> {
        Param::Borrowed(unsafe { mem::transmute(self) })
    }
}

unsafe impl<const T: u128> Interface for IAudioPolicyConfig<T> {
    const IID: GUID = GUID::from_u128(T);
    type Vtable = IAudioPolicyConfig_Vtbl;
}

#[repr(C)]
#[doc(hidden)]
pub struct IAudioPolicyConfig_Vtbl {
    pub base: IInspectableVtbl,

    pub __incomplete__add_CtxVolumeChanged: unsafe extern "system" fn() -> u32,
    pub __incomplete__remove_CtxVolumeChanged: unsafe extern "system" fn() -> u32,
    pub __incomplete__add_RingerVibrateStateChanged: unsafe extern "system" fn() -> u32,
    pub __incomplete__remove_RingerVibrateStateChanged: unsafe extern "system" fn() -> u32,
    pub __incomplete__SetVolumeGroupGainForId: unsafe extern "system" fn() -> u32,
    pub __incomplete__GetVolumeGroupGainForId: unsafe extern "system" fn() -> u32,
    pub __incomplete__GetActiveVolumeGroupForEndpointId: unsafe extern "system" fn() -> u32,
    pub __incomplete__GetVolumeGroupsForEndpoint: unsafe extern "system" fn() -> u32,
    pub __incomplete__GetCurrentVolumeContext: unsafe extern "system" fn() -> u32,
    pub __incomplete__SetVolumeGroupMuteForId: unsafe extern "system" fn() -> u32,
    pub __incomplete__GetVolumeGroupMuteForId: unsafe extern "system" fn() -> u32,
    pub __incomplete__SetRingerVibrateState: unsafe extern "system" fn() -> u32,
    pub __incomplete__GetRingerVibrateState: unsafe extern "system" fn() -> u32,
    pub __incomplete__SetPreferredChatApplication: unsafe extern "system" fn() -> u32,
    pub __incomplete__ResetPreferredChatApplication: unsafe extern "system" fn() -> u32,
    pub __incomplete__GetPreferredChatApplication: unsafe extern "system" fn() -> u32,
    pub __incomplete__GetCurrentChatApplications: unsafe extern "system" fn() -> u32,
    pub __incomplete__add_ChatContextChanged: unsafe extern "system" fn() -> u32,
    pub __incomplete__remove_ChatContextChanged: unsafe extern "system" fn() -> u32,

    pub SetPersistedDefaultAudioEndpoint: unsafe extern "system" fn(
        this: *mut c_void,
        process_id: u32,
        data_flow: EDataFlow,
        role: ERole,
        device_id: HSTRING,
    ) -> HRESULT,

    pub GetPersistedDefaultAudioEndpoint: unsafe extern "system" fn(
        this: *mut c_void,
        process_id: u32,
        data_flow: EDataFlow,
        role: ERole,
        device_id_ptr: *mut ManuallyDrop<HSTRING>,
    ) -> HRESULT,

    pub ClearAllPersistedApplicationDefaultEndpoints:
        unsafe extern "system" fn(this: *mut c_void) -> HRESULT,
}

/// Direction in which audio is moving.
#[derive(Debug)]
pub enum FlowDirection {
    /// Audio is being rendered (played).
    Render,
    /// Audio is being captured.
    Capture,
}

/// Audio device role.
#[derive(Debug)]
pub enum Role {
    /// Interaction with the computer.
    Console,
    /// Playing or recording audio content.
    Multimedia,
    /// Voice communications with another person.
    Communications,
}

/// State of the device.
#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum DeviceState {
    /// The audio endpoint device is active. That is, the audio adapter that
    /// connects to the endpoint device is present and enabled. In addition, if
    /// the endpoint device plugs into a jack on the adapter, then the endpoint
    /// device is plugged in.
    Active,
    /// The audio endpoint device is disabled. The user has disabled the device
    /// in the Windows multimedia control panel.
    Disabled,
    /// The audio endpoint device is not present because the audio adapter that
    /// connects to the endpoint device has been removed from the system, or the
    /// user has disabled the adapter device in Device Manager.
    NotPresent,
    /// The audio endpoint device is unplugged. The audio adapter that contains
    /// the jack for the endpoint device is present and enabled, but the
    /// endpoint device is not plugged into the jack. Only a device with
    /// jack-presence detection can be in this state.
    Unplugged,
}

/// A notification about a device change.
#[derive(Debug)]
pub enum DeviceNotification {
    /// The default device has changed.
    DefaultDeviceChanged {
        /// The flow of the device.
        flow_direction: FlowDirection,
        /// The role of the device.
        role: Role,
        /// The device ID.
        default_device_id: String,
    },
    /// A device was added.
    DeviceAdded {
        /// The device ID.
        device_id: String,
    },
    /// A device was removed.
    DeviceRemoved {
        /// The device ID.
        device_id: String,
    },
    /// The state of a device changed.
    StateChanged {
        /// The device ID.
        device_id: String,
        /// The new device state.
        state: DeviceState,
    },
    /// A property changed on the device.
    PropertyChanged {
        /// The device ID.
        device_id: String,
        /// The property fmtid.
        property_key_fmtid: GUID,
        /// The property pid.
        property_key_pid: u32,
    },
}

#[repr(C)]
pub(crate) struct DeviceNotifications {
    _abi: Box<IMMNotificationClient_Vtbl>,
    ref_cnt: u32,
    tx: Sender<DeviceNotification>,
}

impl DeviceNotifications {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(tx: Sender<DeviceNotification>) -> IMMNotificationClient {
        let target = Box::new(Self {
            _abi: Box::new(IMMNotificationClient_Vtbl {
                base__: IUnknownVtbl {
                    QueryInterface: Self::_query_interface,
                    AddRef: Self::_add_ref,
                    Release: Self::_release,
                },
                OnDeviceStateChanged: Self::_on_device_state_changed,
                OnDeviceAdded: Self::_on_device_added,
                OnDeviceRemoved: Self::_on_device_removed,
                OnDefaultDeviceChanged: Self::_on_default_device_changed,
                OnPropertyValueChanged: Self::_on_property_value_changed,
            }),
            ref_cnt: 1,
            tx,
        });

        unsafe {
            let ptr = Box::into_raw(target);
            mem::transmute(ptr)
        }
    }

    fn query_interface(&mut self, iid: &GUID, interface: *mut *const c_void) -> HRESULT {
        if iid == &IAudioSessionEvents::IID || iid == &IUnknown::IID {
            unsafe {
                *interface = self as *mut Self as *mut _;
            }

            self.add_ref();

            S_OK
        } else {
            E_NOINTERFACE
        }
    }

    fn add_ref(&mut self) -> u32 {
        self.ref_cnt += 1;
        self.ref_cnt
    }

    fn release(&mut self) -> u32 {
        self.ref_cnt -= 1;

        if self.ref_cnt == 0 {
            unsafe {
                Box::from_raw(self as *mut Self);
            }
        }

        self.ref_cnt
    }

    fn on_default_device_changed(
        &mut self,
        flow_direction: FlowDirection,
        role: Role,
        default_device_id: String,
    ) {
        self.tx
            .send(DeviceNotification::DefaultDeviceChanged {
                flow_direction,
                role,
                default_device_id,
            })
            .expect("could not send on_default_device_changed");
    }

    fn on_device_added(&mut self, device_id: String) {
        self.tx
            .send(DeviceNotification::DeviceAdded { device_id })
            .expect("could not send on_device_added");
    }

    fn on_device_removed(&mut self, device_id: String) {
        self.tx
            .send(DeviceNotification::DeviceRemoved { device_id })
            .expect("could not send on_device_removed");
    }

    fn on_device_state_changed(&mut self, device_id: String, new_state: DeviceState) {
        self.tx
            .send(DeviceNotification::StateChanged {
                device_id,
                state: new_state,
            })
            .expect("could not send on_device_state_changed");
    }

    fn on_property_value_changed(&mut self, device_id: String, property_key: PROPERTYKEY) {
        self.tx
            .send(DeviceNotification::PropertyChanged {
                device_id,
                property_key_fmtid: property_key.fmtid,
                property_key_pid: property_key.pid,
            })
            .expect("could not send on_property_value_changed");
    }
}

impl DeviceNotifications {
    unsafe extern "system" fn _query_interface(
        this: RawPtr,
        iid: &GUID,
        interface: *mut *const c_void,
    ) -> HRESULT {
        (*(this as *mut Self)).query_interface(iid, interface)
    }

    unsafe extern "system" fn _add_ref(this: RawPtr) -> u32 {
        (*(this as *mut Self)).add_ref()
    }

    unsafe extern "system" fn _release(this: RawPtr) -> u32 {
        (*(this as *mut Self)).release()
    }

    unsafe extern "system" fn _on_default_device_changed(
        this: RawPtr,
        flow: EDataFlow,
        role: ERole,
        default_device_id: PCWSTR,
    ) -> HRESULT {
        let default_device_id = U16CStr::from_ptr_str(default_device_id.0).to_string_lossy();

        #[allow(non_upper_case_globals)]
        let flow = match flow {
            eRender => FlowDirection::Render,
            eCapture => FlowDirection::Capture,
            _ => {
                warn!("got unknown flow direction {:?}", flow);
                return S_OK;
            }
        };

        #[allow(non_upper_case_globals)]
        let role = match role {
            eConsole => Role::Console,
            eMultimedia => Role::Multimedia,
            eCommunications => Role::Communications,
            _ => {
                warn!("got unknown role {:?}", role);
                return S_OK;
            }
        };

        (*(this as *mut Self)).on_default_device_changed(flow, role, default_device_id);

        S_OK
    }

    unsafe extern "system" fn _on_device_added(this: RawPtr, device_id: PCWSTR) -> HRESULT {
        let device_id = U16CStr::from_ptr_str(device_id.0).to_string_lossy();

        (*(this as *mut Self)).on_device_added(device_id);

        S_OK
    }

    unsafe extern "system" fn _on_device_removed(this: RawPtr, device_id: PCWSTR) -> HRESULT {
        let device_id = U16CStr::from_ptr_str(device_id.0).to_string_lossy();

        (*(this as *mut Self)).on_device_removed(device_id);

        S_OK
    }

    unsafe extern "system" fn _on_device_state_changed(
        this: RawPtr,
        device_id: PCWSTR,
        new_state: u32,
    ) -> HRESULT {
        let device_id = U16CStr::from_ptr_str(device_id.0).to_string_lossy();

        let new_state = match new_state {
            DEVICE_STATE_ACTIVE => DeviceState::Active,
            DEVICE_STATE_DISABLED => DeviceState::Disabled,
            DEVICE_STATE_NOTPRESENT => DeviceState::NotPresent,
            DEVICE_STATE_UNPLUGGED => DeviceState::Unplugged,
            _ => {
                warn!("got unknown device state: {:?}", new_state);
                return S_OK;
            }
        };

        (*(this as *mut Self)).on_device_state_changed(device_id, new_state);

        S_OK
    }

    unsafe extern "system" fn _on_property_value_changed(
        this: RawPtr,
        device_id: PCWSTR,
        property_key: PROPERTYKEY,
    ) -> HRESULT {
        let device_id = U16CStr::from_ptr_str(device_id.0).to_string_lossy();

        (*(this as *mut Self)).on_property_value_changed(device_id, property_key);

        S_OK
    }
}

/// An event for a device.
#[derive(Debug)]
pub struct DeviceEvent {
    /// The new volume level, [0, 1].
    pub level: f32,
    /// If the device is muted.
    pub muted: bool,

    /// The volume for each channel.
    pub channel_volumes: Vec<f32>,

    /// An event context, if one exists.
    pub event_context: GUID,
}

impl From<AUDIO_VOLUME_NOTIFICATION_DATA> for DeviceEvent {
    fn from(notification_data: AUDIO_VOLUME_NOTIFICATION_DATA) -> Self {
        let channel_volumes = unsafe {
            std::slice::from_raw_parts(
                &notification_data.afChannelVolumes as *const _,
                notification_data.nChannels as usize,
            )
        };

        Self {
            level: notification_data.fMasterVolume,
            muted: notification_data.bMuted.into(),

            channel_volumes: channel_volumes.to_vec(),

            event_context: notification_data.guidEventContext,
        }
    }
}

/// A notification about an audio session.
#[derive(Debug)]
pub struct AudioSessionNotification {
    /// The session identifier.
    pub session_identifier: String,
    /// The session instance identifier.
    pub session_instance_identifier: String,
}

#[repr(C)]
pub(crate) struct AudioSessionNotifications {
    _abi: Box<IAudioSessionNotification_Vtbl>,
    ref_cnt: u32,
    tx: Sender<AudioSessionNotification>,
}

impl AudioSessionNotifications {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(tx: Sender<AudioSessionNotification>) -> IAudioSessionNotification {
        let target = Box::new(Self {
            _abi: Box::new(IAudioSessionNotification_Vtbl {
                base__: IUnknownVtbl {
                    QueryInterface: Self::_query_interface,
                    AddRef: Self::_add_ref,
                    Release: Self::_release,
                },
                OnSessionCreated: Self::_on_session_created,
            }),
            ref_cnt: 1,
            tx,
        });

        unsafe {
            let ptr = Box::into_raw(target);
            mem::transmute(ptr)
        }
    }

    fn query_interface(&mut self, iid: &GUID, interface: *mut *const c_void) -> HRESULT {
        if iid == &IAudioSessionEvents::IID || iid == &IUnknown::IID {
            unsafe {
                *interface = self as *mut Self as *mut _;
            }

            self.add_ref();

            S_OK
        } else {
            E_NOINTERFACE
        }
    }

    fn add_ref(&mut self) -> u32 {
        self.ref_cnt += 1;
        self.ref_cnt
    }

    fn release(&mut self) -> u32 {
        self.ref_cnt -= 1;

        if self.ref_cnt == 0 {
            unsafe {
                Box::from_raw(self as *mut Self);
            }
        }

        self.ref_cnt
    }

    fn on_session_created(&mut self, new_session: IAudioSessionControl2) {
        let (session_identifier, session_instance_identifier) = unsafe {
            let session_identifier = new_session.GetSessionIdentifier().unwrap_or_default();
            let session_identifier = U16CStr::from_ptr_str(session_identifier.0).to_string_lossy();

            let session_instance_identifier = new_session
                .GetSessionInstanceIdentifier()
                .unwrap_or_default();
            let session_instance_identifier =
                U16CStr::from_ptr_str(session_instance_identifier.0).to_string_lossy();

            (session_identifier, session_instance_identifier)
        };

        self.tx
            .send(AudioSessionNotification {
                session_identifier,
                session_instance_identifier,
            })
            .expect("could not send on_session_created");
    }
}

impl AudioSessionNotifications {
    unsafe extern "system" fn _query_interface(
        this: RawPtr,
        iid: &GUID,
        interface: *mut *const c_void,
    ) -> HRESULT {
        (*(this as *mut Self)).query_interface(iid, interface)
    }

    unsafe extern "system" fn _add_ref(this: RawPtr) -> u32 {
        (*(this as *mut Self)).add_ref()
    }

    unsafe extern "system" fn _release(this: RawPtr) -> u32 {
        (*(this as *mut Self)).release()
    }

    unsafe extern "system" fn _on_session_created(this: RawPtr, new_session: RawPtr) -> HRESULT {
        struct ComObject(RawPtr);
        let obj = ComObject(new_session);
        let sess = (&*(&obj as *const _ as *const IAudioSessionControl)).clone();

        let new_session = if let Ok(control) = sess.cast() {
            control
        } else {
            warn!("could not cast NewSession to IAudioSessionControl2");
            return S_OK;
        };

        (*(this as *mut Self)).on_session_created(new_session);

        S_OK
    }
}

/// The type of change for an audio session.
#[derive(Debug)]
pub enum AudioSessionEvent {
    /// The volume or mute status has changed.
    VolumeChange {
        /// The new volume level, [0, 1].
        level: f32,
        /// If the session is muted.
        muted: bool,
    },
    /// The state of the session has changed.
    StateChange(SessionState),
    /// The session has disconnected.
    Disconnect(SessionDisconnect),
}

/// An audio session state.
#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum SessionState {
    /// The audio session is currently active.
    Active,
    /// The audio session has become inactive.
    Inactive,
    /// The audio session has expired and is no longer valid.
    Expired,
}

/// The reason why an audio session disconnected.
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub enum SessionDisconnect {
    /// The user removed the audio endpoint device.
    DeviceRemoved,
    /// The Windows audio service has stopped.
    ServerShutdown,
    /// The stream format changed for the device that the audio session is
    /// connected to.
    FormatChanged,
    /// The user logged off the Windows Terminal Services (WTS) session that the
    /// audio session was running in.
    SessionLogoff,
    /// The WTS session that the audio session was running in was disconnected.
    SessionDisconnected,
    /// The (shared-mode) audio session was disconnected to make the audio
    /// endpoint device available for an exclusive-mode connection.
    ExclusiveModeOverride,
}

#[repr(C)]
pub(crate) struct AudioSessionEvents {
    _abi: Box<IAudioSessionEvents_Vtbl>,
    ref_cnt: u32,

    tx: Sender<AudioSessionEvent>,
}

impl AudioSessionEvents {
    pub(crate) fn create(tx: Sender<AudioSessionEvent>) -> IAudioSessionEvents {
        let target = Box::new(Self {
            _abi: Box::new(IAudioSessionEvents_Vtbl {
                base__: IUnknownVtbl {
                    QueryInterface: Self::_query_interface,
                    AddRef: Self::_add_ref,
                    Release: Self::_release,
                },
                OnDisplayNameChanged: Self::_on_display_name_changed,
                OnIconPathChanged: Self::_on_icon_path_changed,
                OnSimpleVolumeChanged: Self::_on_simple_volume_changed,
                OnChannelVolumeChanged: Self::_on_channel_volume_changed,
                OnGroupingParamChanged: Self::_on_grouping_param_changed,
                OnStateChanged: Self::_on_state_changed,
                OnSessionDisconnected: Self::_on_session_disconnected,
            }),
            ref_cnt: 1,
            tx,
        });

        unsafe {
            let ptr = Box::into_raw(target);
            mem::transmute(ptr)
        }
    }

    fn query_interface(&mut self, iid: &GUID, interface: *mut *const c_void) -> HRESULT {
        if iid == &IAudioSessionEvents::IID || iid == &IUnknown::IID {
            unsafe {
                *interface = self as *mut Self as *mut _;
            }

            self.add_ref();

            S_OK
        } else {
            E_NOINTERFACE
        }
    }

    fn add_ref(&mut self) -> u32 {
        self.ref_cnt += 1;
        self.ref_cnt
    }

    fn release(&mut self) -> u32 {
        self.ref_cnt -= 1;

        if self.ref_cnt == 0 {
            unsafe {
                Box::from_raw(self as *mut Self);
            }
        }

        self.ref_cnt
    }

    fn simple_volume_changed(&mut self, new_volume: f32, new_mute: bool) {
        self.tx
            .send(AudioSessionEvent::VolumeChange {
                level: new_volume,
                muted: new_mute,
            })
            .expect("could not send simple_volume_changed");
    }

    fn on_state_changed(&mut self, state: SessionState) {
        self.tx
            .send(AudioSessionEvent::StateChange(state))
            .expect("could not send on_state_changed");
    }

    fn on_session_disconnected(&mut self, session_disconnect: SessionDisconnect) {
        self.tx
            .send(AudioSessionEvent::Disconnect(session_disconnect))
            .expect("could not send on_session_disconnected");
    }
}

/// Methods called by Windows API.
impl AudioSessionEvents {
    unsafe extern "system" fn _query_interface(
        this: RawPtr,
        iid: &GUID,
        interface: *mut *const c_void,
    ) -> HRESULT {
        (*(this as *mut Self)).query_interface(iid, interface)
    }

    unsafe extern "system" fn _add_ref(this: RawPtr) -> u32 {
        (*(this as *mut Self)).add_ref()
    }

    unsafe extern "system" fn _release(this: RawPtr) -> u32 {
        (*(this as *mut Self)).release()
    }

    unsafe extern "system" fn _on_display_name_changed(
        _this: RawPtr,
        _new_display_name: PCWSTR,
        _event_context: *const GUID,
    ) -> HRESULT {
        S_OK
    }

    unsafe extern "system" fn _on_icon_path_changed(
        _this: RawPtr,
        _new_icon_path: PCWSTR,
        _event_context: *const GUID,
    ) -> HRESULT {
        S_OK
    }

    unsafe extern "system" fn _on_simple_volume_changed(
        this: RawPtr,
        new_volume: f32,
        new_mute: BOOL,
        _event_context: *const GUID,
    ) -> HRESULT {
        (*(this as *mut Self)).simple_volume_changed(new_volume, new_mute.into());

        S_OK
    }

    unsafe extern "system" fn _on_channel_volume_changed(
        _this: RawPtr,
        _channel_count: u32,
        _new_channel_volume_array: *const f32,
        _changed_channel: u32,
        _event_context: *const GUID,
    ) -> HRESULT {
        S_OK
    }

    unsafe extern "system" fn _on_grouping_param_changed(
        _this: RawPtr,
        _new_grouping_param: *const GUID,
        _event_context: *const GUID,
    ) -> HRESULT {
        S_OK
    }

    unsafe extern "system" fn _on_state_changed(
        this: RawPtr,
        new_state: AudioSessionState,
    ) -> HRESULT {
        #[allow(non_upper_case_globals)]
        let state = match new_state {
            AudioSessionStateActive => SessionState::Active,
            AudioSessionStateInactive => SessionState::Inactive,
            AudioSessionStateExpired => SessionState::Expired,
            _ => {
                warn!("got unknown state");
                return S_OK;
            }
        };

        (*(this as *mut Self)).on_state_changed(state);

        S_OK
    }

    unsafe extern "system" fn _on_session_disconnected(
        this: RawPtr,
        disconnect_reason: AudioSessionDisconnectReason,
    ) -> HRESULT {
        #[allow(non_upper_case_globals)]
        let session_disconnect = match disconnect_reason {
            DisconnectReasonDeviceRemoval => SessionDisconnect::DeviceRemoved,
            DisconnectReasonServerShutdown => SessionDisconnect::ServerShutdown,
            DisconnectReasonFormatChanged => SessionDisconnect::FormatChanged,
            DisconnectReasonSessionLogoff => SessionDisconnect::SessionLogoff,
            DisconnectReasonSessionDisconnected => SessionDisconnect::SessionDisconnected,
            DisconnectReasonExclusiveModeOverride => SessionDisconnect::ExclusiveModeOverride,
            _ => {
                warn!("got unknown disconnect reason");
                return S_OK;
            }
        };

        (*(this as *mut Self)).on_session_disconnected(session_disconnect);

        S_OK
    }
}
