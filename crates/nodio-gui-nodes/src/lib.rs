#![deny(clippy::all)]
///!
///! Heavily modified clone of [egui_nodes](https://github.com/haighcam/egui_nodes)
///!
use std::cmp::Ordering;
use std::collections::HashMap;

use derivative::Derivative;
use egui::{pos2, Pos2, Rect, Sense, Ui, Vec2};
use indexmap::IndexMap;
use log::debug;
use uuid::Uuid;

use link::*;
use node::*;
use pin::*;

pub use {
    link::LinkArgs,
    node::NodeBuilder,
    pin::{AttributeFlags, PinArgs, PinShape},
    style::{ColorStyle, Style, StyleFlags, StyleVar},
};

mod link;
mod node;
mod pin;
mod style;

/// The Context that tracks the state of the node editor
#[derive(Derivative)]
#[derivative(Default, Debug)]
pub struct Context {
    #[derivative(Debug = "ignore")]
    io: IO,
    #[derivative(Debug = "ignore")]
    style: Style,

    node_ids_overlapping_with_mouse: Vec<Uuid>,
    occluded_pin_ids: Vec<Uuid>,

    canvas_origin_screen_space: Vec2,
    #[derivative(Default(value = "[[0.0; 2].into(); 2].into()"))]
    canvas_rect_screen_space: Rect,

    hovered_node_id: Option<Uuid>,
    interactive_node_id: Option<Uuid>,
    hovered_link_id: Option<Uuid>,
    hovered_pin_id: Option<Uuid>,
    detached_link_id: Option<Uuid>,
    dropped_link_id: Option<Uuid>,
    snap_link_id: Option<Uuid>,

    hovered_pin_flags: usize,
    ui_element_hovered: bool,

    element_state_change: usize,

    active_attribute_id: Uuid,
    active_attribute: bool,

    mouse_pos: Pos2,
    mouse_delta: Vec2,

    left_mouse_pressed: bool,
    left_mouse_released: bool,
    alt_mouse_clicked: bool,
    left_mouse_dragging: bool,
    alt_mouse_dragging: bool,
    mouse_in_canvas: bool,
    link_detach_with_modifier_click: bool,

    nodes: IndexMap<Uuid, Node>,
    pins: IndexMap<Uuid, PinData>,
    links: IndexMap<Uuid, LinkData>,

    end_pin_link_mapping: HashMap<Uuid, Vec<Uuid>>,

    panning: Vec2,

    selected_node_ids: Vec<Uuid>,
    selected_link_ids: Vec<Uuid>,

    node_depth_order: Vec<Uuid>,

    partial_link: Option<(Uuid, Option<Uuid>)>,

    #[derivative(Default(value = "ClickInteractionType::None"))]
    click_interaction_type: ClickInteractionType,
    click_interaction_state: ClickInteractionState,
}

impl Context {
    pub fn begin_frame(&mut self, ui: &mut Ui) {
        self.hovered_node_id.take();
        self.interactive_node_id.take();
        self.hovered_link_id.take();
        self.hovered_pin_flags = AttributeFlags::None as usize;
        self.detached_link_id.take();
        self.dropped_link_id.take();
        self.snap_link_id.take();
        self.partial_link.take();
        self.end_pin_link_mapping.clear();
        self.node_ids_overlapping_with_mouse.clear();
        self.element_state_change = ElementStateChange::None as usize;
        self.active_attribute = false;
        self.canvas_rect_screen_space = ui.available_rect_before_wrap();
        self.canvas_origin_screen_space = self.canvas_rect_screen_space.min.to_vec2();

        for node in self.nodes.values_mut() {
            node.in_use = false;
        }
        for pin in self.pins.values_mut() {
            pin.in_use = false;
        }
        for link in self.links.values_mut() {
            link.in_use = false;
        }

        ui.set_min_size(self.canvas_rect_screen_space.size());

        let mut ui = ui.child_ui(
            self.canvas_rect_screen_space,
            egui::Layout::top_down(egui::Align::Center),
        );

        let screen_rect = ui.ctx().input().screen_rect();
        ui.set_clip_rect(self.canvas_rect_screen_space.intersect(screen_rect));

        ui.painter().rect_filled(
            self.canvas_rect_screen_space,
            0.0,
            self.style.colors[ColorStyle::GridBackground as usize],
        );

        if (self.style.flags & StyleFlags::GridLines as usize) != 0 {
            self.draw_grid(self.canvas_rect_screen_space.size(), &mut ui);
        }
    }

    pub fn end_frame(&mut self, ui: &mut Ui) -> egui::Response {
        let response = ui.interact(
            self.canvas_rect_screen_space,
            ui.id().with("Input"),
            Sense::click_and_drag(),
        );

        let mouse_pos = if let Some(mouse_pos) = response.hover_pos() {
            self.mouse_in_canvas = true;
            mouse_pos
        } else {
            self.mouse_in_canvas = false;
            self.mouse_pos
        };
        self.mouse_delta = mouse_pos - self.mouse_pos;
        self.mouse_pos = mouse_pos;

        let left_mouse_pressed = ui
            .ctx()
            .input()
            .pointer
            .button_down(egui::PointerButton::Primary);
        self.left_mouse_released =
            (self.left_mouse_pressed || self.left_mouse_dragging) && !left_mouse_pressed;
        self.left_mouse_dragging =
            (self.left_mouse_pressed || self.left_mouse_dragging) && left_mouse_pressed;
        self.left_mouse_pressed =
            left_mouse_pressed && !(self.left_mouse_pressed || self.left_mouse_dragging);

        let alt_mouse_clicked = self
            .io
            .emulate_three_button_mouse
            .is_active(&ui.ctx().input().modifiers)
            || self
                .io
                .alt_mouse_button
                .map_or(false, |x| ui.ctx().input().pointer.button_down(x));
        self.alt_mouse_dragging =
            (self.alt_mouse_clicked || self.alt_mouse_dragging) && alt_mouse_clicked;
        self.alt_mouse_clicked =
            alt_mouse_clicked && !(self.alt_mouse_clicked || self.alt_mouse_dragging);
        self.link_detach_with_modifier_click = self
            .io
            .link_detach_with_modifier_click
            .is_active(&ui.ctx().input().modifiers);

        if self.mouse_in_canvas {
            self.resolve_occluded_pins();
            self.resolve_hovered_pin();

            if self.hovered_pin_id.is_none() {
                self.resolve_hovered_node();
            }
        }

        self.click_interaction_update(ui);

        if self.mouse_in_canvas && self.hovered_node_id.is_none() {
            self.resolve_hovered_link();
        }

        for node_id in self.node_depth_order.clone() {
            self.draw_node(node_id, ui);
        }

        let link_ids = self.links.keys().cloned().collect::<Vec<_>>();
        for link_id in link_ids {
            self.draw_link(link_id, ui);
        }

        if self.left_mouse_pressed || self.alt_mouse_clicked {
            self.begin_canvas_interaction();
        }

        self.nodes.retain(|node_id, node| {
            if node.in_use {
                node.pin_ids.clear();
                true
            } else {
                self.node_depth_order.retain(|id| id != node_id);
                false
            }
        });

        self.pins.retain(|_, pin| pin.in_use);
        self.links.retain(|_, link| link.in_use);

        ui.painter().rect_stroke(
            self.canvas_rect_screen_space,
            0.0,
            (1.0, self.style.colors[ColorStyle::GridLine as usize]),
        );

        response
    }

    pub fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }

    pub fn node_pos(&self, node_id: Uuid) -> Option<Pos2> {
        self.nodes.get(&node_id).map(|node| node.origin)
    }

    /// Check if there is a node that is hovered by the pointer
    pub fn hovered_node(&self) -> Option<Uuid> {
        self.hovered_node_id
    }

    /// Check if there is a link that is hovered by the pointer
    pub fn hovered_link(&self) -> Option<Uuid> {
        self.hovered_link_id
    }

    /// Check if there is a pin that is hovered by the pointer
    pub fn hovered_pin(&self) -> Option<Uuid> {
        self.hovered_pin_id
    }

    pub fn num_selected_nodes(&self) -> usize {
        self.selected_link_ids.len()
    }

    pub fn get_selected_nodes(&self) -> &[Uuid] {
        &self.selected_node_ids
    }

    pub fn get_selected_links(&self) -> &[Uuid] {
        &self.selected_link_ids
    }

    pub fn clear_node_selection(&mut self) {
        self.selected_node_ids.clear()
    }

    pub fn clear_link_selection(&mut self) {
        self.selected_link_ids.clear()
    }

    pub fn active_attribute(&self) -> Option<Uuid> {
        if self.active_attribute {
            Some(self.active_attribute_id)
        } else {
            None
        }
    }

    /// Has a new link been created from a pin?
    pub fn started_link_pin(&self) -> Option<Uuid> {
        if (self.element_state_change & ElementStateChange::LinkStarted as usize) != 0 {
            Some(self.click_interaction_state.link_creation.start_pin_id)
        } else {
            None
        }
    }

    /// Has a detached link been dropped?
    pub fn dropped_link(&self) -> Option<Uuid> {
        self.dropped_link_id
    }

    pub fn created_link(&self) -> Option<(Uuid, Uuid, bool)> {
        if (self.element_state_change & ElementStateChange::LinkCreated as usize) != 0 {
            let mut start_pin_id = self.click_interaction_state.link_creation.start_pin_id;
            let mut end_pin_id = self
                .click_interaction_state
                .link_creation
                .end_pin_id
                .unwrap();

            if self.pins.get(&start_pin_id).unwrap().kind != AttributeKind::Output {
                std::mem::swap(&mut start_pin_id, &mut end_pin_id);
            }

            let created_from_snap =
                self.click_interaction_type == ClickInteractionType::LinkCreation;

            Some((start_pin_id, end_pin_id, created_from_snap))
        } else {
            None
        }
    }

    pub fn detached_link(&self) -> Option<Uuid> {
        self.detached_link_id
    }

    pub fn panning(&self) -> Vec2 {
        self.panning
    }

    pub fn reset_panning(&mut self, panning: Vec2) {
        self.panning = panning;
    }

    pub fn node_dimensions(&self, id: Uuid) -> Option<Vec2> {
        self.nodes.iter().find_map(|(&node_id, node)| {
            if node_id == id {
                Some(node.rect.size())
            } else {
                None
            }
        })
    }
}

impl Context {
    pub fn add_node(&mut self, id: Uuid) -> NodeBuilder {
        NodeBuilder::new(self, id)
    }

    pub(crate) fn show_node<'a>(
        &'a mut self,
        NodeBuilder {
            id: node_id,
            header_contents,
            attributes,
            pos,
            ..
        }: NodeBuilder<'a>,
        ui: &mut Ui,
    ) {
        let node: &mut Node = self.nodes.entry(node_id).or_insert_with(|| {
            let mut node = Node::new();
            if let Some(pos) = pos {
                node.origin = pos;
            }
            debug!(
                "New node created at ({}, {}): {}",
                node.origin.x, node.origin.y, node_id
            );

            if !self.node_depth_order.contains(&node_id) {
                self.node_depth_order.push(node_id);
            }
            node
        });
        node.in_use = true;

        self.style.format_node(node);
        node.background_shape
            .replace(ui.painter().add(egui::Shape::Noop));

        let node_origin = node.origin;
        let node_size = node.size;
        let title_space = node.layout_style.padding.y;

        node.header_shapes.push(ui.painter().add(egui::Shape::Noop));
        node.header_shapes.push(ui.painter().add(egui::Shape::Noop));
        let mut header_content_rect = node.header_content_rect;

        let padding = node.layout_style.padding;
        let node_pos = self.grid_space_to_screen_space(node_origin);

        let response = ui.allocate_ui_at_rect(Rect::from_min_size(node_pos, node_size), |ui| {
            if let Some(header_contents) = header_contents {
                let response = ui.allocate_ui(ui.available_size(), header_contents);
                header_content_rect = response.response.rect;

                ui.add_space(title_space);
            }

            ui.allocate_space(Vec2::splat(4.0));

            for NodeAttribute {
                id: attr_id,
                kind,
                pin_args,
                add_contents,
            } in attributes
            {
                let response = ui.allocate_ui(ui.available_size(), add_contents);
                let response = response.response.union(response.inner);
                self.add_attribute(attr_id, node_id, kind, pin_args, response);
            }

            ui.rect_contains_pointer(ui.min_rect().expand2(padding))
        });

        let node: &mut Node = self.nodes.get_mut(&node_id).unwrap();

        node.rect = response.response.rect.expand2(padding);
        node.header_content_rect = header_content_rect.expand2(padding);

        node.header_content_rect.max.x = node.rect.max.x;

        let hovered = response.inner;
        if hovered {
            self.node_ids_overlapping_with_mouse.push(node_id);
        }
    }

    fn add_attribute(
        &mut self,
        pin_id: Uuid,
        node_id: Uuid,
        kind: AttributeKind,
        args: PinArgs,
        response: egui::Response,
    ) {
        if kind != AttributeKind::None {
            let pin = self.pins.entry(pin_id).or_default();
            pin.in_use = true;
            pin.parent_node_id = node_id;
            pin.kind = kind;
            pin.attribute_rect = response.rect;

            self.style.format_pin(pin, args);
            self.nodes.get_mut(&node_id).unwrap().add_pin(pin_id);
        }

        if response.is_pointer_button_down_on() {
            self.active_attribute = true;
            self.active_attribute_id = pin_id;
            self.interactive_node_id.replace(node_id);
        }
    }

    pub fn add_link(
        &mut self,
        id: Uuid,
        start_pin_id: Uuid,
        end_pin_id: Uuid,
        args: LinkArgs,
        ui: &mut Ui,
    ) {
        self.end_pin_link_mapping
            .entry(end_pin_id)
            .or_default()
            .push(id);

        let link: &mut LinkData = self.links.entry(id).or_default();
        link.in_use = true;
        link.start_pin_id = start_pin_id;
        link.end_pin_id = end_pin_id;

        link.shape.replace(ui.painter().add(egui::Shape::Noop));
        self.style.format_link(link, args);

        if (self.click_interaction_type == ClickInteractionType::LinkCreation
            && self
                .pins
                .get(&link.end_pin_id)
                .unwrap()
                .link_creation_on_snap_enabled()
            && self.click_interaction_state.link_creation.start_pin_id == link.start_pin_id
            && self.click_interaction_state.link_creation.end_pin_id == Some(link.end_pin_id))
            || (self.click_interaction_state.link_creation.start_pin_id == link.end_pin_id
                && self.click_interaction_state.link_creation.end_pin_id == Some(link.start_pin_id))
        {
            self.snap_link_id.replace(id);
        }
    }

    fn draw_grid(&self, canvas_size: Vec2, ui: &mut Ui) {
        let mut y = self.panning.y.rem_euclid(self.style.grid_spacing);
        while y < canvas_size.y {
            let mut x = self.panning.x.rem_euclid(self.style.grid_spacing);
            while x < canvas_size.x {
                ui.painter().circle_filled(
                    self.editor_space_to_screen_space([x, y].into()),
                    2.0,
                    self.style.colors[ColorStyle::GridLine as usize],
                );
                x += self.style.grid_spacing;
            }

            y += self.style.grid_spacing;
        }
    }

    fn grid_space_to_screen_space(&self, v: Pos2) -> Pos2 {
        v + self.canvas_origin_screen_space + self.panning
    }

    fn editor_space_to_screen_space(&self, v: Pos2) -> Pos2 {
        v + self.canvas_origin_screen_space
    }

    fn get_screen_space_pin_coordinates(&self, pin: &PinData) -> Pos2 {
        let parent_node_rect = self.nodes.get(&pin.parent_node_id).unwrap().rect;
        self.style.get_screen_space_pin_coordinates(
            &parent_node_rect,
            &pin.attribute_rect,
            pin.kind,
        )
    }

    fn resolve_occluded_pins(&mut self) {
        self.occluded_pin_ids.clear();

        let depth_stack = &self.node_depth_order;
        if depth_stack.len() < 2 {
            return;
        }

        for depth_idx in 0..(depth_stack.len() - 1) {
            let node_below = self.nodes.get(&depth_stack[depth_idx]).unwrap();
            for next_depth in &depth_stack[(depth_idx + 1)..(depth_stack.len())] {
                let rect_above = self.nodes.get(next_depth).unwrap().rect;
                for pin_id in node_below.pin_ids.iter() {
                    let pin_pos = self.pins.get(pin_id).unwrap().pos;
                    if rect_above.contains(pin_pos) {
                        self.occluded_pin_ids.push(*pin_id);
                    }
                }
            }
        }
    }

    fn resolve_hovered_pin(&mut self) {
        let mut smallest_distance = f32::MAX;
        self.hovered_pin_id.take();

        let hover_radius_sqr = self.style.pin_hover_radius.powi(2);

        for (pin_id, pin) in self.pins.iter() {
            if self.occluded_pin_ids.contains(pin_id) {
                continue;
            }

            let distance_sqr = (pin.pos - self.mouse_pos).length_sq();
            if distance_sqr < hover_radius_sqr && distance_sqr < smallest_distance {
                smallest_distance = distance_sqr;
                self.hovered_pin_id.replace(*pin_id);
            }
        }
    }

    fn resolve_hovered_node(&mut self) {
        match self.node_ids_overlapping_with_mouse.len() {
            0 => {
                self.hovered_node_id.take();
            }
            1 => {
                self.hovered_node_id
                    .replace(self.node_ids_overlapping_with_mouse[0]);
            }
            _ => {
                let mut largest_depth_idx = -1;

                for node_id in self.node_ids_overlapping_with_mouse.iter() {
                    for (depth_idx, depth_node_id) in self.node_depth_order.iter().enumerate() {
                        if *depth_node_id == *node_id && depth_idx as isize > largest_depth_idx {
                            largest_depth_idx = depth_idx as isize;
                            self.hovered_node_id.replace(*node_id);
                        }
                    }
                }
            }
        }
    }

    fn resolve_hovered_link(&mut self) {
        let mut smallest_distance = f32::MAX;
        self.hovered_link_id.take();

        let links_clone = self.links.clone();
        for (&link_id, link) in self.links.iter_mut() {
            if !self.pins.contains_key(&link.start_pin_id)
                || !self.pins.contains_key(&link.end_pin_id)
            {
                continue;
            }

            let start_pin = self.pins.get(&link.start_pin_id).unwrap();
            let end_pin = self.pins.get(&link.end_pin_id).unwrap();

            let pin_link_count = Self::link_count_for_end_pin(
                &self.end_pin_link_mapping,
                link.end_pin_id,
                &self.partial_link,
            );
            let idx = Self::link_index_for_end_pin(
                &self.end_pin_link_mapping,
                &links_clone,
                &self.pins,
                &self.partial_link,
                link.end_pin_id,
                link_id,
                start_pin.pos,
            )
            .unwrap_or(0);

            let end_pos = if self.hovered_pin_id == Some(link.end_pin_id) && pin_link_count > 1 {
                self.style
                    .calculate_link_end_pos(end_pin.pos, self.mouse_pos, pin_link_count, idx)
            } else {
                end_pin.pos
            };

            let link_data = LinkBezierData::build(
                start_pin.pos,
                end_pos,
                start_pin.kind,
                self.style.link_line_segments_per_length,
            );

            let distance = link_data.get_distance_to_cubic_bezier(&self.mouse_pos);

            if distance < self.style.link_hover_distance && distance < smallest_distance {
                smallest_distance = distance;
                self.hovered_link_id.replace(link_id);
            }
        }
    }

    fn link_count_for_end_pin(
        end_pin_links: &HashMap<Uuid, Vec<Uuid>>,
        pin_id: Uuid,
        partial_link: &Option<(Uuid, Option<Uuid>)>,
    ) -> usize {
        let mut count = end_pin_links
            .get(&pin_id)
            .map(|links| links.len())
            .unwrap_or(0);

        match partial_link {
            Some((start_pin_id, Some(partial_link_end_pin_id)))
                if *partial_link_end_pin_id == pin_id && *start_pin_id != pin_id =>
            {
                count += 1;
            }
            _ => {}
        }

        count
    }

    fn link_index_for_end_pin(
        end_pin_links: &HashMap<Uuid, Vec<Uuid>>,
        links: &IndexMap<Uuid, LinkData>,
        pins: &IndexMap<Uuid, PinData>,
        partial_link: &Option<(Uuid, Option<Uuid>)>,
        pin_id: Uuid,
        link_id: Uuid,
        start_pos: Pos2,
    ) -> Option<usize> {
        let end_pin_pos = pins.get(&pin_id)?.pos;

        let link_ids = end_pin_links.get(&pin_id)?;
        let mut link_angles = link_ids
            .iter()
            .filter(|&id| *id != link_id)
            .filter_map(|link_id| links.get(link_id).map(|link| (*link_id, link.start_pin_id)))
            .filter_map(|(link_id, pin_id)| pins.get(&pin_id).map(|pin| (link_id, pin.pos)))
            .chain([(link_id, start_pos)])
            .map(|(link_id, start_pin_pos)| (link_id, (end_pin_pos - start_pin_pos).angle()))
            .collect::<Vec<_>>();

        match partial_link {
            Some((start_pin_id, Some(end_pin_id)))
                if *end_pin_id == pin_id && *start_pin_id != pin_id =>
            {
                link_angles.push((
                    Uuid::nil(),
                    (end_pin_pos - pins.get(start_pin_id).unwrap().pos).angle(),
                ));
            }
            _ => {}
        }

        link_angles.sort_by(|(_, angle1), (_, angle2)| {
            angle2.partial_cmp(angle1).unwrap_or(Ordering::Equal)
        });

        link_angles.iter().position(|(id, _)| *id == link_id)
    }

    fn draw_link(&mut self, link_id: Uuid, ui: &mut Ui) {
        let links_clone = self.links.clone();
        let link = self.links.get_mut(&link_id).unwrap();

        if !link.in_use
            || !self.pins.contains_key(&link.start_pin_id)
            || !self.pins.contains_key(&link.end_pin_id)
        {
            return;
        }

        let same_pin_link_count = Self::link_count_for_end_pin(
            &self.end_pin_link_mapping,
            link.end_pin_id,
            &self.partial_link,
        );
        let idx = Self::link_index_for_end_pin(
            &self.end_pin_link_mapping,
            &links_clone,
            &self.pins,
            &self.partial_link,
            link.end_pin_id,
            link_id,
            self.pins.get(&link.start_pin_id).unwrap().pos,
        )
        .unwrap_or(0);

        let start_pin = self.pins.get(&link.start_pin_id).unwrap();
        let end_pin = self.pins.get(&link.end_pin_id).unwrap();
        let hovered_pin_id = self.hovered_pin_id;

        let end_pos = if hovered_pin_id == Some(link.end_pin_id) && same_pin_link_count > 1 {
            self.style
                .calculate_link_end_pos(end_pin.pos, self.mouse_pos, same_pin_link_count, idx)
        } else {
            end_pin.pos
        };

        let link_bezier_data = LinkBezierData::build(
            start_pin.pos,
            end_pos,
            start_pin.kind,
            self.style.link_line_segments_per_length,
        );
        let link_shape = link.shape.take().unwrap();
        let link_hovered = self.hovered_link_id == Some(link_id)
            && self.click_interaction_type != ClickInteractionType::BoxSelection;

        if link_hovered && self.left_mouse_pressed {
            self.begin_link_interaction(link_id);
        }

        if self.detached_link_id == Some(link_id) {
            return;
        }

        let link = self.links.get(&link_id).unwrap();
        let mut link_color = link.color_style.base;
        if self.partial_link.is_none() {
            if self.selected_link_ids.contains(&link_id) {
                link_color = link.color_style.selected;
            } else if link_hovered {
                link_color = link.color_style.hovered;
            }
        }

        ui.painter().set(
            link_shape,
            link_bezier_data.draw((self.style.link_thickness, link_color)),
        );
    }

    fn draw_node(&mut self, node_id: Uuid, ui: &mut Ui) {
        let node: &mut Node = self.nodes.get_mut(&node_id).unwrap();
        if !node.in_use {
            return;
        }

        let node_hovered = self.hovered_node_id == Some(node_id)
            && self.click_interaction_type != ClickInteractionType::BoxSelection;

        let (node_bg_color, title_bg_color) = if self.selected_node_ids.contains(&node_id) {
            (
                node.color_style.background_selected,
                node.color_style.header_selected,
            )
        } else if node_hovered {
            (
                node.color_style.background_hovered,
                node.color_style.header_hovered,
            )
        } else {
            (node.color_style.background, node.color_style.header)
        };

        let painter = ui.painter();

        if let Some(bg_shape) = node.background_shape.take() {
            painter.set(
                bg_shape,
                egui::Shape::rect_filled(
                    node.rect,
                    node.layout_style.corner_rounding,
                    node_bg_color,
                ),
            );
        }

        if node.header_content_rect.height() > 0.0 {
            if let Some(title_shape) = node.header_shapes.pop() {
                painter.set(
                    title_shape,
                    egui::Shape::rect_filled(
                        Rect::from_min_size(
                            node.header_content_rect.min,
                            Vec2::new(
                                node.header_content_rect.width(),
                                node.layout_style.corner_rounding * 2.0,
                            ),
                        ),
                        node.layout_style.corner_rounding,
                        title_bg_color,
                    ),
                );
            }

            if let Some(title_shape) = node.header_shapes.pop() {
                painter.set(
                    title_shape,
                    egui::Shape::rect_filled(
                        Rect::from_min_size(
                            node.header_content_rect.min
                                + Vec2::new(0.0, node.layout_style.corner_rounding),
                            node.header_content_rect.size()
                                - Vec2::new(0.0, node.layout_style.corner_rounding),
                        ),
                        0.0,
                        title_bg_color,
                    ),
                );
            }
        }

        for pin_id in node.pin_ids.iter().cloned().collect::<Vec<_>>() {
            self.draw_pin(pin_id, ui);
        }

        if node_hovered && self.left_mouse_pressed && self.interactive_node_id != Some(node_id) {
            self.begin_node_selection(node_id);
        }
    }

    fn draw_pin(&mut self, pin_id: Uuid, ui: &mut Ui) {
        let pin: &mut PinData = self.pins.get_mut(&pin_id).unwrap();
        let parent_node_rect = self.nodes.get(&pin.parent_node_id).unwrap().rect;

        pin.pos = self.style.get_screen_space_pin_coordinates(
            &parent_node_rect,
            &pin.attribute_rect,
            pin.kind,
        );

        let mut pin_color = pin.color_style.background;

        let pin_hovered = self.hovered_pin_id == Some(pin_id)
            && self.click_interaction_type != ClickInteractionType::BoxSelection;
        let pin_shape = pin.shape;
        let pin_pos = pin.pos;

        let attached_link_count =
            Self::link_count_for_end_pin(&self.end_pin_link_mapping, pin_id, &self.partial_link);

        if pin_hovered {
            self.hovered_pin_flags = pin.flags;
            pin_color = pin.color_style.hovered;

            if self.left_mouse_pressed && (pin.is_output() || self.hovered_link_id.is_some()) {
                self.begin_link_creation(pin_id);
            }
        }

        if pin_hovered && attached_link_count > 1 {
            self.style.draw_hovered_pin(
                attached_link_count,
                pin_pos,
                self.mouse_pos,
                pin_shape,
                pin_color,
                ui,
            );
        } else {
            self.style.draw_pin(
                pin_pos,
                pin_shape,
                pin_color,
                self.style.pin_circle_radius,
                ui,
            );
        }
    }

    fn begin_canvas_interaction(&mut self) {
        let any_ui_element_hovered = self.hovered_node_id.is_some()
            || self.hovered_link_id.is_some()
            || self.hovered_pin_id.is_some();

        let mouse_not_in_canvas = !self.mouse_in_canvas;

        if self.click_interaction_type != ClickInteractionType::None
            || any_ui_element_hovered
            || mouse_not_in_canvas
        {
            return;
        }

        if self.alt_mouse_clicked {
            self.click_interaction_type = ClickInteractionType::Panning;
        } else {
            self.click_interaction_type = ClickInteractionType::BoxSelection;
            self.click_interaction_state.box_selection.min = self.mouse_pos;
        }
    }

    fn translate_selected_nodes(&mut self) {
        if self.left_mouse_dragging {
            let delta = self.mouse_delta;
            for node_id in self.selected_node_ids.iter() {
                let node = self.nodes.get_mut(node_id).unwrap();
                if node.draggable {
                    node.origin += delta;
                }
            }
        }
    }

    fn should_link_snap_to_pin(
        &self,
        start_pin: &PinData,
        hovered_pin_id: Uuid,
        duplicate_link: Option<Uuid>,
    ) -> bool {
        let end_pin = self.pins.get(&hovered_pin_id).unwrap();

        if start_pin.parent_node_id == end_pin.parent_node_id {
            return false;
        }

        if start_pin.kind == end_pin.kind {
            return false;
        }

        if duplicate_link.map_or(false, |duplicate_id| {
            Some(duplicate_id) != self.snap_link_id
        }) {
            return false;
        }

        true
    }

    fn box_selector_update_selection(&mut self) -> Rect {
        let mut box_rect = self.click_interaction_state.box_selection;
        if box_rect.min.x > box_rect.max.x {
            std::mem::swap(&mut box_rect.min.x, &mut box_rect.max.x);
        }

        if box_rect.min.y > box_rect.max.y {
            std::mem::swap(&mut box_rect.min.y, &mut box_rect.max.y);
        }

        self.selected_node_ids.clear();
        for (node_id, node) in self.nodes.iter() {
            if node.in_use && box_rect.intersects(node.rect) {
                self.selected_node_ids.push(*node_id);
            }
        }

        self.selected_link_ids.clear();
        for (&link_id, link) in self.links.iter().filter(|(_, link)| link.in_use) {
            if !self.pins.contains_key(&link.start_pin_id)
                || !self.pins.contains_key(&link.end_pin_id)
            {
                continue;
            }

            let pin_start = self.pins.get(&link.start_pin_id).unwrap();
            let pin_end = self.pins.get(&link.end_pin_id).unwrap();
            let node_start_rect = self.nodes.get(&pin_start.parent_node_id).unwrap().rect;
            let node_end_rect = self.nodes.get(&pin_end.parent_node_id).unwrap().rect;
            let start = self.style.get_screen_space_pin_coordinates(
                &node_start_rect,
                &pin_start.attribute_rect,
                pin_start.kind,
            );
            let end = self.style.get_screen_space_pin_coordinates(
                &node_end_rect,
                &pin_end.attribute_rect,
                pin_end.kind,
            );

            if self.rectangle_overlaps_link(&box_rect, &start, &end, pin_start.kind) {
                self.selected_link_ids.push(link_id);
            }
        }
        box_rect
    }

    #[inline]
    fn rectangle_overlaps_link(
        &self,
        rect: &Rect,
        start: &Pos2,
        end: &Pos2,
        start_type: AttributeKind,
    ) -> bool {
        let mut lrect = Rect::from_min_max(*start, *end);
        if lrect.min.x > lrect.max.x {
            std::mem::swap(&mut lrect.min.x, &mut lrect.max.x);
        }

        if lrect.min.y > lrect.max.y {
            std::mem::swap(&mut lrect.min.y, &mut lrect.max.y);
        }

        if rect.intersects(lrect) {
            if rect.contains(*start) || rect.contains(*end) {
                return true;
            }

            let link_data = LinkBezierData::build(
                *start,
                *end,
                start_type,
                self.style.link_line_segments_per_length,
            );
            return link_data.rectangle_overlaps_bezier(rect);
        }
        false
    }

    fn click_interaction_update(&mut self, ui: &mut Ui) {
        match self.click_interaction_type {
            ClickInteractionType::BoxSelection => {
                self.click_interaction_state.box_selection.max = self.mouse_pos;
                let rect = self.box_selector_update_selection();

                let box_selector_color = self.style.colors[ColorStyle::BoxSelector as usize];
                let box_selector_outline =
                    self.style.colors[ColorStyle::BoxSelectorOutline as usize];
                ui.painter()
                    .rect(rect, 0.0, box_selector_color, (1.0, box_selector_outline));

                if self.left_mouse_released {
                    let mut ids = Vec::with_capacity(self.selected_node_ids.len());
                    let depth_stack = &mut self.node_depth_order;
                    let selected_nodes = &self.selected_node_ids;
                    depth_stack.retain(|id| {
                        if selected_nodes.contains(id) {
                            ids.push(*id);
                            false
                        } else {
                            true
                        }
                    });
                    self.node_depth_order.extend(ids);
                    self.click_interaction_type = ClickInteractionType::None;
                }
            }
            ClickInteractionType::Node => {
                self.translate_selected_nodes();
                if self.left_mouse_released {
                    self.click_interaction_type = ClickInteractionType::None;
                }
            }
            ClickInteractionType::Link => {
                if self.left_mouse_released {
                    self.click_interaction_type = ClickInteractionType::None;
                }
            }
            ClickInteractionType::LinkCreation => {
                let maybe_duplicate_link_id = self.hovered_pin_id.and_then(|hovered_pin_id| {
                    self.find_duplicate_link(
                        self.click_interaction_state.link_creation.start_pin_id,
                        hovered_pin_id,
                    )
                });

                let should_snap = self.hovered_pin_id.map_or(false, |hovered_pin_id| {
                    let start_pin = self
                        .pins
                        .get(&self.click_interaction_state.link_creation.start_pin_id)
                        .unwrap();
                    self.should_link_snap_to_pin(start_pin, hovered_pin_id, maybe_duplicate_link_id)
                });

                let snapping_pin_changed = self
                    .click_interaction_state
                    .link_creation
                    .end_pin_id
                    .map_or(false, |pin_id| self.hovered_pin_id != Some(pin_id));

                if snapping_pin_changed && self.snap_link_id.is_some() {
                    self.begin_link_detach(
                        self.snap_link_id.unwrap(),
                        self.click_interaction_state
                            .link_creation
                            .end_pin_id
                            .unwrap(),
                    );
                }

                let start_pin = self
                    .pins
                    .get(&self.click_interaction_state.link_creation.start_pin_id)
                    .unwrap();
                let start_pos = self.get_screen_space_pin_coordinates(start_pin);

                self.partial_link = Some((
                    self.click_interaction_state.link_creation.start_pin_id,
                    self.hovered_pin_id,
                ));

                let end_pos = if should_snap {
                    let hovered_pin_id = self.hovered_pin_id.unwrap();

                    let pin_pos = self
                        .get_screen_space_pin_coordinates(self.pins.get(&hovered_pin_id).unwrap());

                    let same_pin_link_count = Self::link_count_for_end_pin(
                        &self.end_pin_link_mapping,
                        hovered_pin_id,
                        &self.partial_link,
                    );

                    let idx = Self::link_index_for_end_pin(
                        &self.end_pin_link_mapping,
                        &self.links,
                        &self.pins,
                        &self.partial_link,
                        hovered_pin_id,
                        Uuid::nil(),
                        start_pos,
                    )
                    .unwrap_or(0);

                    if same_pin_link_count > 1 {
                        self.style.calculate_link_end_pos(
                            pin_pos,
                            self.mouse_pos,
                            same_pin_link_count,
                            idx,
                        )
                    } else {
                        pin_pos
                    }
                } else {
                    self.mouse_pos
                };

                let link_data = LinkBezierData::build(
                    start_pos,
                    end_pos,
                    start_pin.kind,
                    self.style.link_line_segments_per_length,
                );
                ui.painter().add(link_data.draw((
                    self.style.link_thickness,
                    self.style.colors[ColorStyle::Link as usize],
                )));

                let link_creation_on_snap = self.hovered_pin_id.map_or(false, |hovered_pin_id| {
                    self.pins
                        .get(&hovered_pin_id)
                        .unwrap()
                        .link_creation_on_snap_enabled()
                });

                if !should_snap {
                    self.click_interaction_state.link_creation.end_pin_id.take();
                }

                let create_link =
                    should_snap && (self.left_mouse_released || link_creation_on_snap);

                if create_link && maybe_duplicate_link_id.is_none() {
                    if !self.left_mouse_released
                        && self.click_interaction_state.link_creation.end_pin_id
                            == self.hovered_pin_id
                    {
                        return;
                    }
                    self.element_state_change |= ElementStateChange::LinkCreated as usize;
                    self.click_interaction_state.link_creation.end_pin_id = self.hovered_pin_id;
                }

                if self.left_mouse_released {
                    self.click_interaction_type = ClickInteractionType::None;
                    if !create_link {
                        self.element_state_change |= ElementStateChange::LinkDropped as usize;
                        if self
                            .click_interaction_state
                            .link_creation
                            .link_creation_type
                            == LinkCreationType::FromDetach
                        {
                            self.dropped_link_id =
                                self.click_interaction_state.link_creation.detached_link_id;
                        }
                    }
                }
            }
            ClickInteractionType::Panning => {
                if self.alt_mouse_dragging || self.alt_mouse_clicked {
                    self.panning += self.mouse_delta;
                } else {
                    self.click_interaction_type = ClickInteractionType::None;
                }
            }
            ClickInteractionType::None => (),
        }
    }

    fn begin_link_detach(&mut self, id: Uuid, detach_id: Uuid) {
        self.click_interaction_state.link_creation.end_pin_id.take();

        let link = self.links.get(&id).unwrap();
        self.click_interaction_state.link_creation.start_pin_id = if detach_id == link.start_pin_id
        {
            link.end_pin_id
        } else {
            link.start_pin_id
        };
        self.detached_link_id.replace(id);
    }

    fn begin_link_interaction(&mut self, id: Uuid) {
        let link = self.links.get(&id).unwrap();

        if self.click_interaction_type == ClickInteractionType::LinkCreation {
            if (self.hovered_pin_flags & AttributeFlags::EnableLinkDetachWithDragClick as usize)
                != 0
            {
                self.begin_link_detach(id, self.hovered_pin_id.unwrap());
                self.click_interaction_state.link_creation.detached_link_id = Some(id);
                self.click_interaction_state
                    .link_creation
                    .link_creation_type = LinkCreationType::FromDetach;
            }
        } else if self.link_detach_with_modifier_click {
            let start_pin = self.pins.get(&link.start_pin_id).unwrap();
            let end_pin = self.pins.get(&link.end_pin_id).unwrap();
            let dist_to_start = start_pin.pos.distance(self.mouse_pos);
            let dist_to_end = end_pin.pos.distance(self.mouse_pos);
            let closest_pin_idx = if dist_to_start < dist_to_end {
                link.start_pin_id
            } else {
                link.end_pin_id
            };
            self.click_interaction_type = ClickInteractionType::LinkCreation;
            self.begin_link_detach(id, closest_pin_idx);
        } else {
            self.begin_link_selection(id);
        }
    }

    fn begin_link_creation(&mut self, hovered_pin_id: Uuid) {
        self.click_interaction_type = ClickInteractionType::LinkCreation;
        self.click_interaction_state.link_creation.start_pin_id = hovered_pin_id;
        self.click_interaction_state.link_creation.end_pin_id.take();
        self.click_interaction_state
            .link_creation
            .link_creation_type = LinkCreationType::Standard;
        self.element_state_change |= ElementStateChange::LinkStarted as usize;
    }

    fn begin_link_selection(&mut self, link_id: Uuid) {
        self.click_interaction_type = ClickInteractionType::Link;
        self.selected_node_ids.clear();
        self.selected_link_ids.clear();
        self.selected_link_ids.push(link_id);
    }

    fn find_duplicate_link(&self, start_pin_id: Uuid, end_pin_id: Uuid) -> Option<Uuid> {
        self.links.iter().find_map(|(&link_id, link)| {
            if link.in_use && link.start_pin_id == start_pin_id && link.end_pin_id == end_pin_id {
                Some(link_id)
            } else {
                None
            }
        })
    }

    fn begin_node_selection(&mut self, id: Uuid) {
        if self.click_interaction_type != ClickInteractionType::None {
            return;
        }
        self.click_interaction_type = ClickInteractionType::Node;
        if !self.selected_node_ids.contains(&id) {
            self.selected_node_ids.clear();
            self.selected_link_ids.clear();
            self.selected_node_ids.push(id);

            self.node_depth_order.retain(|depth_id| *depth_id != id);
            self.node_depth_order.push(id);
        }
    }
}

#[derive(Debug)]
enum ElementStateChange {
    None = 0,
    LinkStarted = 1 << 0,
    LinkDropped = 1 << 1,
    LinkCreated = 1 << 2,
}

#[derive(PartialEq, Debug, Copy, Clone)]
enum ClickInteractionType {
    Node,
    Link,
    LinkCreation,
    Panning,
    BoxSelection,
    None,
}

#[derive(PartialEq, Debug)]
enum LinkCreationType {
    Standard,
    FromDetach,
}

#[derive(Derivative, Debug)]
#[derivative(Default)]
struct ClickInteractionStateLinkCreation {
    start_pin_id: Uuid,
    end_pin_id: Option<Uuid>,
    detached_link_id: Option<Uuid>,
    #[derivative(Default(value = "LinkCreationType::Standard"))]
    link_creation_type: LinkCreationType,
}

#[derive(Derivative, Debug)]
#[derivative(Default)]
struct ClickInteractionState {
    link_creation: ClickInteractionStateLinkCreation,
    #[derivative(Default(value = "[[0.0; 2].into(); 2].into()"))]
    box_selection: Rect,
}

/// This controls the modifiers needed for certain mouse interactions
#[derive(Derivative, Debug)]
#[derivative(Default)]
pub struct IO {
    /// The Modifier that needs to pressed to pan the editor
    #[derivative(Default(value = "Modifiers::None"))]
    pub emulate_three_button_mouse: Modifiers,

    // The modifier that needs to be pressed to detach a link instead of creating a new one
    #[derivative(Default(value = "Modifiers::None"))]
    pub link_detach_with_modifier_click: Modifiers,

    // The mouse button that pans the editor. Should probably not be set to Primary.
    #[derivative(Default(value = "Some(egui::PointerButton::Middle)"))]
    pub alt_mouse_button: Option<egui::PointerButton>,
}

/// Used to track which Egui Modifier needs to be pressed for certain IO actions
#[derive(Debug)]
pub enum Modifiers {
    Alt,
    Ctrl,
    Shift,
    Command,
    None,
}

impl Modifiers {
    fn is_active(&self, mods: &egui::Modifiers) -> bool {
        match self {
            Modifiers::Alt => mods.alt,
            Modifiers::Ctrl => mods.ctrl,
            Modifiers::Shift => mods.shift,
            Modifiers::Command => mods.command,
            Modifiers::None => false,
        }
    }
}
