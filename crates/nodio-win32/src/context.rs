use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use log::{debug, error, info, trace, warn};
use notify_thread::JoinHandle;
use parking_lot::RwLock;
use windows::core::HSTRING;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eMultimedia, eRender, EDataFlow, DEVICE_STATEMASK_ALL,
};
use windows::Win32::System::Threading::GetCurrentProcessId;

use nodio_core::{Context, DeviceInfo, Node, NodeKind, ProcessInfo, Uuid};
use nodio_core::{Error, Result};

use crate::com::ensure_com_initialized;
use crate::custom::{
    create_audio_policy_config, AudioPolicyConfig, AudioSessionEvent, SessionState,
};
use crate::device::{
    AudioDevice, DEVINTERFACE_AUDIO_CAPTURE, DEVINTERFACE_AUDIO_RENDER, MMDEVAPI_TOKEN,
};
use crate::enumerator::AudioDeviceEnumerator;
use crate::loopback::LoopbackSession;
use crate::node::{NodeConnectionInfo, NodeConnectionKind};
use crate::session::{session_node_match, AudioSession, AudioSessionKind};

pub struct Win32Context {
    device_enumerator: AudioDeviceEnumerator,
    audio_policy_config: Box<dyn AudioPolicyConfig>,

    nodes: Vec<Node>,

    node_connections: Vec<NodeConnectionInfo>,

    loopback_sessions: Arc<RwLock<Vec<LoopbackSession>>>,

    sessions: Arc<RwLock<Vec<AudioSession>>>,
    input_devices: Arc<RwLock<Vec<AudioDevice>>>,
    output_devices: Arc<RwLock<Vec<AudioDevice>>>,

    session_update_thread: Option<JoinHandle<()>>,
}

unsafe impl Send for Win32Context {}
unsafe impl Sync for Win32Context {}

impl Drop for Win32Context {
    fn drop(&mut self) {
        self.session_update_thread.take().and_then(|thread| {
            thread.notify();
            thread.join().ok()
        });
    }
}

impl Win32Context {
    pub fn new() -> Arc<RwLock<Self>> {
        ensure_com_initialized();

        let device_enumerator = AudioDeviceEnumerator::create().unwrap();

        let ctx = Arc::new(RwLock::new(Win32Context {
            device_enumerator,
            audio_policy_config: create_audio_policy_config(),
            nodes: vec![],
            sessions: Default::default(),
            input_devices: Default::default(),
            output_devices: Default::default(),
            node_connections: Default::default(),
            loopback_sessions: Default::default(),
            session_update_thread: None,
        }));

        let mut output_devices = ctx
            .read()
            .device_enumerator
            .enumerate_audio_endpoints(eRender, DEVICE_STATEMASK_ALL)
            .unwrap();

        let mut input_devices = ctx
            .read()
            .device_enumerator
            .enumerate_audio_endpoints(eCapture, DEVICE_STATEMASK_ALL)
            .unwrap();

        for device in input_devices.iter_mut().chain(output_devices.iter_mut()) {
            let ctx = ctx.clone();
            let name = device.name().to_string();

            device.set_session_notification_callback(move |event| {
                trace!("Session notification in {}: {:?}", name, event);

                Self::refresh_sessions(ctx.clone());
            });
        }

        ctx.write().input_devices = Arc::new(RwLock::new(input_devices));
        ctx.write().output_devices = Arc::new(RwLock::new(output_devices));

        Self::refresh_sessions(ctx.clone());

        let session_update_thread = {
            let ctx = ctx.clone();

            notify_thread::spawn(move |thread| {
                trace!("Session update thread started");

                while !thread.notified() {
                    let sessions: Arc<_> = ctx.read().sessions.clone();
                    let input_devices: Arc<_> = ctx.read().input_devices.clone();
                    let output_devices: Arc<_> = ctx.read().output_devices.clone();

                    for session in sessions.read().iter() {
                        if let Some(node) = ctx
                            .write()
                            .nodes
                            .iter_mut()
                            .find(|n| session_node_match(n, session))
                        {
                            node.process_id = Some(session.process_id());
                            node.peak_values = session.peak_values().unwrap_or((0.0, 0.0));
                            node.volume = session.master_volume();
                            node.active = session.is_active();
                            node.present = true;
                        }
                    }

                    for device in input_devices
                        .read()
                        .iter()
                        .chain(output_devices.read().iter())
                    {
                        if let Some(node) =
                            ctx.write().nodes.iter_mut().find(|n| n.id == device.id())
                        {
                            node.peak_values = device.peak_values().unwrap_or((0.0, 0.0));
                            node.volume = device.master_volume();
                            node.active = device.is_active();
                            node.present = true;
                        }
                    }

                    thread::sleep(Duration::from_secs_f32(1.0 / 30.0));
                }

                trace!("Session update thread stopped");
            })
        };

        ctx.write().session_update_thread = Some(session_update_thread);

        ctx
    }

    fn refresh_sessions(ctx: Arc<RwLock<Win32Context>>) {
        debug!("Refreshing sessions");

        let mut sessions = Vec::new();

        for device in ctx
            .read()
            .output_devices
            .read()
            .iter()
            .filter(|d| d.is_active())
        {
            let device_sessions = match device.enumerate_sessions() {
                Ok(sessions) => sessions,
                Err(err) => {
                    error!("Failed to get device sessions: {}", err);
                    continue;
                }
            };

            for mut session in device_sessions {
                let pid = session.process_id();
                if pid == 0 {
                    continue;
                }

                session.set_event_callback({
                    let ctx = ctx.clone();
                    let session = session.clone();

                    move |event| {
                        trace!("Session event: {:?}", event);
                        match event {
                            AudioSessionEvent::VolumeChange { level, muted } => {
                                if let Some(node) = ctx
                                    .write()
                                    .nodes
                                    .iter_mut()
                                    .find(|n| session_node_match(n, &session))
                                {
                                    node.volume = level;
                                    node.muted = muted;
                                }
                            }
                            AudioSessionEvent::StateChange(state) => {
                                if let Some(node) = ctx
                                    .write()
                                    .nodes
                                    .iter_mut()
                                    .find(|n| session_node_match(n, &session))
                                {
                                    node.peak_values = (0.0, 0.0);
                                    node.active = state == SessionState::Active;
                                    node.present = state != SessionState::Expired;
                                }
                            }
                            AudioSessionEvent::Disconnect(reason) => {
                                trace!("Session disconnected. Reason: {:?}", reason);

                                ctx.write()
                                    .sessions
                                    .write()
                                    .retain(|s| s.id() != session.id());
                            }
                        }
                    }
                });

                sessions.push(session);
            }
        }

        ctx.write().sessions = Arc::new(RwLock::new(sessions));
    }

    fn parse_mmdevice_id(mmdevice_id: &str) -> Option<(Uuid, EDataFlow)> {
        mmdevice_id
            .split(MMDEVAPI_TOKEN)
            .nth(1)
            .and_then(|s| s.split_once('#'))
            .and_then(|(device_id, data_flow)| {
                let device_id = device_id.to_string();
                let device_id = Uuid::from_str(&device_id[1..device_id.len() - 1]).unwrap();

                match data_flow {
                    DEVINTERFACE_AUDIO_RENDER => Some((device_id, eRender)),
                    DEVINTERFACE_AUDIO_CAPTURE => Some((device_id, eCapture)),
                    _ => None,
                }
            })
    }

    fn get_default_audio_endpoint_for_process(
        &self,
        process_id: u32,
    ) -> windows::core::Result<(Uuid, EDataFlow)> {
        let device_id = unsafe {
            self.audio_policy_config.persistent_default_audio_endpoint(
                process_id,
                eRender,
                eMultimedia,
            )?
        };

        Ok(Self::parse_mmdevice_id(&device_id.to_string()).unwrap_or((Uuid::nil(), eRender)))
    }

    fn use_system_default_audio_endpoint_for_process(
        &self,
        process_id: u32,
    ) -> windows::core::Result<()> {
        self.set_default_audio_endpoint_for_process(process_id, HSTRING::new())
    }

    fn set_default_audio_endpoint_for_process(
        &self,
        process_id: u32,
        device_id: HSTRING,
    ) -> windows::core::Result<()> {
        unsafe {
            self.audio_policy_config
                .set_persistent_default_audio_endpoint(
                    process_id,
                    eRender,
                    eMultimedia,
                    device_id.clone(),
                )?;

            self.audio_policy_config
                .set_persistent_default_audio_endpoint(process_id, eRender, eConsole, device_id)
        }
    }

    fn connect_application_node(&mut self, node_id: Uuid, target_id: Uuid) -> Result<()> {
        let node = self.nodes.iter().find(|n| n.id == node_id).unwrap();

        if node.process_id.is_none() {
            return Err(Error::CouldNotConnect("No such process".to_string()));
        }

        let output_devices = self.output_devices.read();
        let target_device = output_devices.iter().find(|d| d.id() == target_id).unwrap();

        let mut conn_info = NodeConnectionInfo {
            id: Uuid::new_v4(),
            src_id: node_id,
            dst_id: target_id,
            kind: NodeConnectionKind::DefaultEndpoint,
        };

        if self
            .node_connections
            .iter()
            .any(|conn| conn.src_id == node_id)
        {
            info!("Already connected, using loopback for stream duplication");

            let loopback_session = LoopbackSession::start(
                node_id,
                target_id,
                node.process_id.unwrap(),
                target_device.mmdevice(),
            )
            .map_err(|err| {
                error!("Could not start loopback session: {}", err);
                Error::CouldNotConnect(err.to_string())
            })?;

            conn_info.kind = NodeConnectionKind::Loopback;

            self.loopback_sessions.write().push(loopback_session);
        } else if let Some(session) = self
            .sessions
            .read()
            .iter()
            .find(|session| session_node_match(node, session))
        {
            match self.get_default_audio_endpoint_for_process(session.process_id()) {
                Ok((device_id, _)) => {
                    if device_id != target_device.id() {
                        if let Err(err) = self.set_default_audio_endpoint_for_process(
                            session.process_id(),
                            target_device.mmdevice_id(eRender),
                        ) {
                            error!(
                                "Failed to set audio endpoint for process {}: {:?}",
                                session.process_id(),
                                err
                            );
                            return Err(Error::CouldNotConnect(err.to_string()));
                        } else {
                            debug!(
                                "Set default audio endpoint for process {}",
                                session.process_id()
                            );
                        }
                    } else {
                        debug!("Endpoint is already the same");
                    }
                }
                Err(err) => {
                    error!(
                        "Failed to get default endpoint for process {}: {}",
                        session.process_id(),
                        err
                    );
                }
            }
        }

        self.node_connections.push(conn_info);

        Ok(())
    }

    fn connect_input_device(&mut self, node_id: Uuid, target_id: Uuid) -> Result<()> {
        let input_devices = self.input_devices.write();
        let output_devices = self.output_devices.read();

        let input_device = input_devices
            .iter()
            .find(|device| device.id() == node_id)
            .ok_or_else(|| Error::CouldNotConnect("no such input device found".to_string()))?;

        let output_device = output_devices
            .iter()
            .find(|device| device.id() == target_id)
            .ok_or_else(|| Error::CouldNotConnect("no such output device found".to_string()))?;

        if let Err(err) = input_device.set_listen(Some(output_device)) {
            warn!(
                "Failed to enable listening on device {}: {}",
                input_device.name(),
                err
            );
            return Err(Error::CouldNotConnect(err.to_string()));
        }

        self.node_connections.push(NodeConnectionInfo {
            id: Uuid::new_v4(),
            src_id: node_id,
            dst_id: target_id,
            kind: NodeConnectionKind::Listen,
        });

        Ok(())
    }

    fn output_device_exists(&self, id: Uuid) -> bool {
        self.output_devices.read().iter().any(|d| d.id() == id)
    }
}

impl Context for Win32Context {
    fn add_node(&mut self, mut node: Node) {
        if self.nodes.iter().any(|other| other.id == node.id) {
            info!("Node already added: {}", &node.display_name);
            return;
        }

        if let Some(session) = self
            .sessions
            .read()
            .iter()
            .find(|&session| session_node_match(&node, session))
        {
            node.process_id = Some(session.process_id());
        }

        self.nodes.push(node);
    }

    fn remove_node(&mut self, node_id: Uuid) {
        let connections = self
            .node_connections
            .iter()
            .filter(|conn| conn.src_id == node_id || conn.dst_id == node_id)
            .copied()
            .collect::<Vec<_>>();

        for conn in connections {
            self.disconnect_node(conn.src_id, conn.dst_id);
        }

        self.nodes.retain(|node| node.id != node_id);
    }

    fn nodes(&self) -> &[Node] {
        self.nodes.as_slice()
    }

    fn nodes_mut(&mut self) -> &mut [Node] {
        &mut self.nodes
    }

    fn connect_node(&mut self, node_id: Uuid, target_id: Uuid) -> Result<()> {
        let node_kind = match self.nodes.iter().find(|n| n.id == node_id) {
            Some(node) => node.kind,
            None => {
                warn!("No node found for id {}", node_id);
                return Err(Error::CouldNotConnect("No such node found".to_string()));
            }
        };

        if !self.output_device_exists(target_id) {
            warn!("No output device found for node id: {}", target_id);
            return Err(Error::NoSuchDevice);
        }

        match node_kind {
            NodeKind::Application => self.connect_application_node(node_id, target_id)?,
            NodeKind::InputDevice => self.connect_input_device(node_id, target_id)?,

            NodeKind::OutputDevice => {
                warn!("Output device cannot be used as an input!");
                return Err(Error::CouldNotConnect(
                    "Output device cannot be used as an input!".to_string(),
                ));
            }
        }

        Ok(())
    }

    fn disconnect_node(&mut self, src_id: Uuid, dst_id: Uuid) {
        let removed_connection = match self
            .node_connections
            .iter()
            .position(|conn| conn.src_id == src_id && conn.dst_id == dst_id)
            .map(|idx| self.node_connections.remove(idx))
        {
            Some(conn) => conn,
            None => {
                warn!("No such connection found");
                return;
            }
        };

        info!("Removed connection {} => {}", src_id, dst_id);

        let node = match self.nodes.iter().find(|node| node.id == src_id) {
            Some(node) => node,
            None => {
                warn!("No such node found");
                return;
            }
        };

        match node.kind {
            NodeKind::Application => {
                if node.process_id.is_none() {
                    return;
                }

                match removed_connection.kind {
                    NodeConnectionKind::DefaultEndpoint => {
                        let next_src_connection = self
                            .node_connections
                            .iter_mut()
                            .find(|conn| conn.src_id == src_id);

                        if let Some(next_conn) = next_src_connection {
                            if next_conn.kind == NodeConnectionKind::Loopback {
                                self.loopback_sessions.write().retain(|s| {
                                    s.src_id != next_conn.src_id || s.dst_id != next_conn.dst_id
                                });
                            }

                            next_conn.kind = NodeConnectionKind::DefaultEndpoint;

                            let target_mmdevice_id = self
                                .output_devices
                                .read()
                                .iter()
                                .find(|d| d.id() == next_conn.dst_id)
                                .map(|d| d.mmdevice_id(eRender))
                                .unwrap();

                            self.set_default_audio_endpoint_for_process(
                                node.process_id.unwrap(),
                                target_mmdevice_id,
                            )
                            .ok();
                        } else {
                            self.use_system_default_audio_endpoint_for_process(
                                node.process_id.unwrap(),
                            )
                            .ok();
                        }
                    }
                    NodeConnectionKind::Loopback => {
                        self.loopback_sessions
                            .write()
                            .retain(|s| s.src_id != src_id || s.dst_id != dst_id);
                    }
                    _ => {}
                }
            }

            NodeKind::InputDevice => {
                if let Some(device) = self
                    .input_devices
                    .write()
                    .iter_mut()
                    .find(|device| device.id() == src_id)
                {
                    if let Err(err) = device.set_listen(None) {
                        warn!(
                            "Failed to enable listening on device {}: {}",
                            &device.name(),
                            err
                        )
                    }
                } else {
                    warn!("No input device found for id {}", src_id);
                }
            }
            _ => {}
        }
    }

    fn set_volume(&mut self, node_id: Uuid, volume: f32) {
        if let Some(node) = self.nodes.iter().find(|n| n.id == node_id) {
            for matching_session in self
                .sessions
                .read()
                .iter()
                .filter(|session| session_node_match(node, session))
            {
                matching_session.set_master_volume(volume);
            }

            for matching_device in self
                .output_devices
                .read()
                .iter()
                .filter(|device| device.id() == node_id)
            {
                matching_device.set_master_volume(volume);
            }
        }
    }

    fn application_processes(&self) -> Vec<ProcessInfo> {
        let mut added_pids = HashSet::new();
        let mut processes = Vec::new();

        let my_pid = unsafe { GetCurrentProcessId() };

        for session in self
            .sessions
            .read()
            .iter()
            .filter(|s| s.kind() == AudioSessionKind::Application && s.process_id() != my_pid)
        {
            if added_pids.insert(session.process_id()) {
                processes.push(ProcessInfo {
                    pid: session.process_id(),
                    display_name: session.display_name().to_string(),
                    filename: session.filename().to_string(),
                });
            }
        }

        processes
    }

    fn input_devices(&self) -> Vec<DeviceInfo> {
        self.input_devices
            .read()
            .iter()
            .filter(|d| !d.id().is_nil() && d.is_active())
            .map(|d| DeviceInfo {
                id: d.id(),
                name: d.name().to_string(),
            })
            .collect::<Vec<_>>()
    }

    fn output_devices(&self) -> Vec<DeviceInfo> {
        self.output_devices
            .read()
            .iter()
            .filter(|d| !d.id().is_nil() && d.is_active())
            .map(|d| DeviceInfo {
                id: d.id(),
                name: d.name().to_string(),
            })
            .collect::<Vec<_>>()
    }
}
