#![deny(clippy::all)]
mod result;
pub use result::{Error, Result};

use serde::{Deserialize, Serialize};
pub use uuid::Uuid;

pub trait Context {
    fn add_node(&mut self, node: Node);
    fn remove_node(&mut self, node_id: Uuid);
    fn nodes(&self) -> &[Node];
    fn nodes_mut(&mut self) -> &mut [Node];
    fn connect_node(&mut self, node_id: Uuid, target_id: Uuid) -> Result<()>;
    fn disconnect_node(&mut self, node_id: Uuid, target_id: Uuid);
    fn set_volume(&mut self, node_id: Uuid, volume: f32);
    fn application_processes(&self) -> Vec<ProcessInfo>;
    fn input_devices(&self) -> Vec<DeviceInfo>;
    fn output_devices(&self) -> Vec<DeviceInfo>;
}

#[derive(Debug, Clone, PartialOrd, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: Uuid,
    pub kind: NodeKind,
    pub display_name: String,
    pub filename: String,

    pub pos: (f32, f32),

    #[serde(skip)]
    pub process_id: Option<u32>,
    #[serde(skip)]
    pub active: bool,
    #[serde(skip)]
    pub present: bool,
    #[serde(skip)]
    pub volume: f32,
    #[serde(skip)]
    pub muted: bool,
    #[serde(skip)]
    pub peak_values: (f32, f32),
}

impl Default for Node {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            kind: NodeKind::Application,
            display_name: String::new(),
            filename: String::new(),
            pos: (0.0, 0.0),
            process_id: None,
            active: false,
            present: false,
            volume: 1.0,
            muted: false,
            peak_values: (0.0, 0.0),
        }
    }
}

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    Application,
    OutputDevice,
    InputDevice,
}

pub struct DeviceInfo {
    pub id: Uuid,
    pub name: String,
}

pub struct ProcessInfo {
    pub pid: u32,
    pub display_name: String,
    pub filename: String,
}
