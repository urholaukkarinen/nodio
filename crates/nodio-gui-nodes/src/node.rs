use super::*;
use derivative::Derivative;
use std::collections::HashSet;

#[derive(Default, Debug)]
pub(crate) struct NodeColorStyle {
    pub background: egui::Color32,
    pub background_hovered: egui::Color32,
    pub background_selected: egui::Color32,
    pub header: egui::Color32,
    pub header_hovered: egui::Color32,
    pub header_selected: egui::Color32,
}

#[derive(Default, Debug)]
pub struct NodeLayoutStyle {
    pub corner_rounding: f32,
    pub padding: Vec2,
    pub border_thickness: f32,
}

#[derive(Derivative)]
#[derivative(Debug)]
pub(crate) struct Node {
    pub in_use: bool,
    pub origin: Pos2,
    pub size: Vec2,
    pub header_content_rect: Rect,
    pub rect: Rect,
    #[derivative(Debug = "ignore")]
    pub color_style: NodeColorStyle,
    pub layout_style: NodeLayoutStyle,
    pub pin_ids: HashSet<Uuid>,
    pub draggable: bool,

    #[derivative(Debug = "ignore")]
    pub header_shapes: Vec<egui::layers::ShapeIdx>,
    #[derivative(Debug = "ignore")]
    pub background_shape: Option<egui::layers::ShapeIdx>,
}

impl Node {
    pub fn new() -> Self {
        Self {
            in_use: true,
            origin: [100.0; 2].into(),
            size: [180.0; 2].into(),
            header_content_rect: [[0.0; 2].into(); 2].into(),
            rect: [[0.0; 2].into(); 2].into(),
            color_style: Default::default(),
            layout_style: Default::default(),
            pin_ids: Default::default(),
            draggable: true,
            header_shapes: Vec::new(),
            background_shape: None,
        }
    }

    pub fn add_pin(&mut self, pin_id: Uuid) {
        self.pin_ids.insert(pin_id);
    }
}

impl Default for Node {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) struct NodeAttribute<'a> {
    pub(crate) id: Uuid,
    pub(crate) kind: AttributeKind,
    pub(crate) pin_args: PinArgs,
    pub(crate) add_contents: Box<dyn FnOnce(&mut Ui) -> egui::Response + 'a>,
}

/// Used to construct a node and stores the relevant ui code for its title and attributes
/// This is used so that the nodes can be rendered in the context depth order
#[derive(Derivative)]
#[derivative(Debug)]
pub struct NodeBuilder<'a> {
    pub(crate) ctx: Option<&'a mut Context>,

    pub(crate) id: Uuid,
    #[derivative(Debug = "ignore")]
    pub(crate) header_contents: Option<Box<dyn FnOnce(&mut Ui) + 'a>>,
    #[derivative(Debug = "ignore")]
    pub(crate) attributes: Vec<NodeAttribute<'a>>,
    pub(crate) pos: Option<Pos2>,
}

impl<'a> NodeBuilder<'a> {
    /// Create a new node to be displayed in a [Context].
    /// Id should be the same across frames and should not be the same as any
    /// other currently used nodes.
    pub fn new(ctx: &'a mut Context, id: Uuid) -> Self {
        Self {
            ctx: Some(ctx),
            id,
            header_contents: None,
            attributes: Vec::new(),
            pos: None,
        }
    }

    /// Add a header with given contents.
    pub fn with_header(mut self, add_contents: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.header_contents.replace(Box::new(add_contents));
        self
    }

    /// Add an input attribute that can be connected to output attributes of other nodes.
    /// Id should be the same across frames and should not be the same as any other currently used
    /// attributes.
    pub fn with_input_attribute(
        &mut self,
        id: Uuid,
        pin_args: PinArgs,
        add_contents: impl FnOnce(&mut Ui) -> egui::Response + 'a,
    ) -> &mut Self {
        self.attributes.push(NodeAttribute {
            id,
            kind: AttributeKind::Input,
            pin_args,
            add_contents: Box::new(add_contents),
        });
        self
    }

    /// Add an output attribute that can be connected to input attributes of other nodes.
    /// Id should be the same across frames and should not be the same as any other currently used
    /// attributes.
    pub fn with_output_attribute(
        &mut self,
        id: Uuid,
        pin_args: PinArgs,
        add_contents: impl FnOnce(&mut Ui) -> egui::Response + 'a,
    ) -> &mut Self {
        self.attributes.push(NodeAttribute {
            id,
            kind: AttributeKind::Output,
            pin_args,
            add_contents: Box::new(add_contents),
        });
        self
    }

    /// Add a static attribute that cannot be connected to other attributes.
    /// Id should be the same across frames and should not be the same as any other currently used
    /// attributes.
    pub fn with_static_attribute(
        mut self,
        id: Uuid,
        add_contents: impl FnOnce(&mut Ui) -> egui::Response + 'a,
    ) -> Self {
        self.attributes.push(NodeAttribute {
            id,
            kind: AttributeKind::None,
            pin_args: PinArgs::default(),
            add_contents: Box::new(add_contents),
        });
        self
    }

    /// Set the position of the node in screen space when it is first created.
    pub fn with_origin(mut self, origin: Pos2) -> Self {
        self.pos.replace(origin);
        self
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn show(mut self, ui: &mut Ui) {
        self.ctx.take().unwrap().show_node(self, ui);
    }
}
