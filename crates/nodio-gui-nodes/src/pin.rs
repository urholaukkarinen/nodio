use super::*;
use derivative::Derivative;

#[derive(Default, Debug)]
pub struct PinArgs {
    pub shape: PinShape,
    pub flags: Option<usize>,
    pub background: Option<egui::Color32>,
    pub hovered: Option<egui::Color32>,
}

impl PinArgs {
    pub const fn new() -> Self {
        Self {
            shape: PinShape::CircleFilled,
            flags: None,
            background: None,
            hovered: None,
        }
    }
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub(crate) enum AttributeKind {
    None,
    Input,
    Output,
}

impl Default for AttributeKind {
    fn default() -> Self {
        Self::None
    }
}

/// Controls the shape of an attribute pin.
#[derive(Clone, Copy, Debug)]
pub enum PinShape {
    Circle,
    CircleFilled,
    Triangle,
    TriangleFilled,
    Quad,
    QuadFilled,
}

impl Default for PinShape {
    fn default() -> Self {
        Self::CircleFilled
    }
}

/// Controls the way that attribute pins behave
#[derive(Debug)]
pub enum AttributeFlags {
    None = 0,

    /// If there is a link on the node then it will detatch instead of creating a new one.
    /// Requires handling of deleted links via Context::link_destroyed
    EnableLinkDetachWithDragClick = 1 << 0,

    /// Visual snapping will trigger link creation / destruction
    EnableLinkCreationOnSnap = 1 << 1,
}

#[derive(Default, Debug)]
pub(crate) struct PinDataColorStyle {
    pub background: egui::Color32,
    pub hovered: egui::Color32,
}

#[derive(Derivative)]
#[derivative(Debug)]
pub(crate) struct PinData {
    pub in_use: bool,
    pub parent_node_id: Uuid,
    pub attribute_rect: Rect,
    pub kind: AttributeKind,
    pub shape: PinShape,
    pub pos: Pos2,
    pub flags: usize,
    #[derivative(Debug = "ignore")]
    pub color_style: PinDataColorStyle,
}

impl Default for PinData {
    fn default() -> Self {
        Self::new()
    }
}

impl PinData {
    pub fn new() -> Self {
        Self {
            in_use: true,
            parent_node_id: Default::default(),
            attribute_rect: [[0.0; 2].into(); 2].into(),
            kind: AttributeKind::None,
            shape: PinShape::CircleFilled,
            pos: Default::default(),
            flags: AttributeFlags::None as usize,
            color_style: Default::default(),
        }
    }

    pub fn is_output(&self) -> bool {
        self.kind == AttributeKind::Output
    }

    pub fn link_creation_on_snap_enabled(&self) -> bool {
        self.flags & AttributeFlags::EnableLinkCreationOnSnap as usize != 0
    }
}
