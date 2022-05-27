use super::*;

use egui::{remap, Pos2};
use std::f32::consts::{FRAC_PI_4, FRAC_PI_8, PI};

/// Represents different color style values used by a Context
#[derive(Debug, Clone, Copy)]
pub enum ColorStyle {
    NodeBackground = 0,
    NodeBackgroundHovered,
    NodeBackgroundSelected,
    NodeHeader,
    NodeHeaderHovered,
    NodeHeaderSelected,
    Link,
    LinkHovered,
    LinkSelected,
    Pin,
    PinHovered,
    BoxSelector,
    BoxSelectorOutline,
    GridBackground,
    GridLine,
    Count,
}

/// Represents different style values used by a Context
#[derive(Debug, Clone, Copy)]
pub enum StyleVar {
    GridSpacing = 0,
    NodeCornerRounding,
    NodePaddingHorizontal,
    NodePaddingVertical,
    NodeBorderThickness,
    LinkThickness,
    LinkLineSegmentsPerLength,
    LinkHoverDistance,
    PinCircleRadius,
    PinQuadSideLength,
    PinTriangleSideLength,
    PinLineThickness,
    PinHoverRadius,
    PinOffset,
}

/// Controls some style aspects
#[derive(Debug)]
pub enum StyleFlags {
    None = 0,
    GridLines = 1 << 2,
}

impl ColorStyle {
    /// dark color style
    pub fn colors_dark() -> [egui::Color32; ColorStyle::Count as usize] {
        let mut colors = [egui::Color32::BLACK; ColorStyle::Count as usize];
        colors[ColorStyle::NodeBackground as usize] =
            egui::Color32::from_rgba_unmultiplied(50, 50, 50, 255);
        colors[ColorStyle::NodeBackgroundHovered as usize] =
            egui::Color32::from_rgba_unmultiplied(75, 75, 75, 255);
        colors[ColorStyle::NodeBackgroundSelected as usize] =
            egui::Color32::from_rgba_unmultiplied(75, 75, 75, 255);
        colors[ColorStyle::NodeHeader as usize] =
            egui::Color32::from_rgba_unmultiplied(74, 74, 74, 255);
        colors[ColorStyle::NodeHeaderHovered as usize] =
            egui::Color32::from_rgba_unmultiplied(94, 94, 94, 255);
        colors[ColorStyle::NodeHeaderSelected as usize] =
            egui::Color32::from_rgba_unmultiplied(120, 120, 120, 255);
        colors[ColorStyle::Link as usize] =
            egui::Color32::from_rgba_unmultiplied(60, 133, 224, 255);
        colors[ColorStyle::LinkHovered as usize] =
            egui::Color32::from_rgba_unmultiplied(60, 150, 250, 255);
        colors[ColorStyle::LinkSelected as usize] =
            egui::Color32::from_rgba_unmultiplied(60, 150, 250, 255);
        colors[ColorStyle::Pin as usize] = egui::Color32::from_rgba_unmultiplied(60, 133, 224, 255);
        colors[ColorStyle::PinHovered as usize] =
            egui::Color32::from_rgba_unmultiplied(53, 150, 250, 255);
        colors[ColorStyle::BoxSelector as usize] =
            egui::Color32::from_rgba_unmultiplied(61, 133, 224, 30);
        colors[ColorStyle::BoxSelectorOutline as usize] =
            egui::Color32::from_rgba_unmultiplied(61, 133, 224, 150);
        colors[ColorStyle::GridBackground as usize] = egui::Color32::from_rgb(20, 20, 20);
        colors[ColorStyle::GridLine as usize] = egui::Color32::from_rgb(26, 26, 26);
        colors
    }
}

#[derive(Debug)]
pub struct Style {
    pub grid_spacing: f32,
    pub node_corner_rounding: f32,
    pub node_padding_horizontal: f32,
    pub node_padding_vertical: f32,
    pub node_border_thickness: f32,

    pub link_thickness: f32,
    pub link_line_segments_per_length: f32,
    pub link_hover_distance: f32,

    pub pin_circle_radius: f32,
    pub pin_quad_side_length: f32,
    pub pin_triangle_side_length: f32,
    pub pin_line_thickness: f32,
    pub pin_hover_radius: f32,
    pub pin_hover_shape_radius: f32,
    pub pin_offset: f32,

    pub flags: usize,
    pub colors: [egui::Color32; ColorStyle::Count as usize],
}

impl Default for Style {
    fn default() -> Self {
        Self {
            grid_spacing: 26.0,
            node_corner_rounding: 4.0,
            node_padding_horizontal: 8.0,
            node_padding_vertical: 8.0,
            node_border_thickness: 1.0,
            link_thickness: 3.0,
            link_line_segments_per_length: 0.1,
            link_hover_distance: 10.0,
            pin_circle_radius: 4.0,
            pin_quad_side_length: 7.0,
            pin_triangle_side_length: 9.5,
            pin_line_thickness: 1.0,
            pin_hover_radius: 25.0,
            pin_hover_shape_radius: 15.0,
            pin_offset: 0.0,
            flags: StyleFlags::GridLines as usize,
            colors: ColorStyle::colors_dark(),
        }
    }
}

impl Style {
    pub(crate) fn get_screen_space_pin_coordinates(
        &self,
        node_rect: &Rect,
        attribute_rect: &Rect,
        kind: AttributeKind,
    ) -> Pos2 {
        let x = match kind {
            AttributeKind::Input => node_rect.min.x - self.pin_offset,
            _ => node_rect.max.x + self.pin_offset,
        };
        egui::pos2(x, 0.5 * (attribute_rect.min.y + attribute_rect.max.y))
    }

    pub(crate) fn draw_hovered_pin(
        &self,
        link_count: usize,
        pin_pos: Pos2,
        mouse_pos: Pos2,
        pin_shape: PinShape,
        pin_color: egui::Color32,
        ui: &mut Ui,
    ) {
        ui.painter().add(egui::Shape::circle_stroke(
            pin_pos,
            self.hovered_pin_radius(pin_pos, mouse_pos),
            (self.pin_line_thickness, pin_color),
        ));

        self.draw_pin(
            pin_pos,
            pin_shape,
            pin_color,
            self.pin_circle_radius / 2.0,
            ui,
        );

        for i in 0..link_count {
            let pin_pos = self.calculate_link_end_pos(pin_pos, mouse_pos, link_count, i);
            self.draw_pin(pin_pos, pin_shape, pin_color, self.pin_circle_radius, ui);
        }
    }

    pub(crate) fn draw_pin(
        &self,
        pin_pos: Pos2,
        pin_shape: PinShape,
        pin_color: egui::Color32,
        pin_radius: f32,
        ui: &mut Ui,
    ) {
        let painter = ui.painter();

        match pin_shape {
            PinShape::Circle => painter.add(egui::Shape::circle_stroke(
                pin_pos,
                pin_radius,
                (self.pin_line_thickness, pin_color),
            )),
            PinShape::CircleFilled => {
                painter.add(egui::Shape::circle_filled(pin_pos, pin_radius, pin_color))
            }
            PinShape::Quad => painter.add(egui::Shape::rect_stroke(
                Rect::from_center_size(pin_pos, [self.pin_quad_side_length / 2.0; 2].into()),
                0.0,
                (self.pin_line_thickness, pin_color),
            )),
            PinShape::QuadFilled => painter.add(egui::Shape::rect_filled(
                Rect::from_center_size(pin_pos, [self.pin_quad_side_length / 2.0; 2].into()),
                0.0,
                pin_color,
            )),
            PinShape::Triangle => {
                let sqrt_3 = 3f32.sqrt();
                let left_offset = -0.166_666_7 * sqrt_3 * self.pin_triangle_side_length;
                let right_offset = 0.333_333_3 * sqrt_3 * self.pin_triangle_side_length;
                let verticacl_offset = 0.5 * self.pin_triangle_side_length;
                painter.add(egui::Shape::closed_line(
                    vec![
                        pin_pos + (left_offset, verticacl_offset).into(),
                        pin_pos + (right_offset, 0.0).into(),
                        pin_pos + (left_offset, -verticacl_offset).into(),
                    ],
                    (self.pin_line_thickness, pin_color),
                ))
            }
            PinShape::TriangleFilled => {
                let sqrt_3 = 3f32.sqrt();
                let left_offset = -0.166_666_7 * sqrt_3 * self.pin_triangle_side_length;
                let right_offset = 0.333_333_3 * sqrt_3 * self.pin_triangle_side_length;
                let verticacl_offset = 0.5 * self.pin_triangle_side_length;
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        pin_pos + (left_offset, verticacl_offset).into(),
                        pin_pos + (right_offset, 0.0).into(),
                        pin_pos + (left_offset, -verticacl_offset).into(),
                    ],
                    pin_color,
                    egui::Stroke::none(),
                ))
            }
        };
    }

    pub(crate) fn hovered_pin_radius(&self, pin_pos: Pos2, mouse_pos: Pos2) -> f32 {
        remap(
            self.pin_hover_radius - (pin_pos - mouse_pos).length(),
            0.0..=(self.pin_hover_radius - self.pin_hover_shape_radius - 5.0),
            0.0..=self.pin_hover_shape_radius,
        )
        .min(self.pin_hover_shape_radius)
    }

    pub(crate) fn calculate_link_end_pos(
        &self,
        pin_pos: Pos2,
        mouse_pos: Pos2,
        link_count: usize,
        link_index: usize,
    ) -> Pos2 {
        let ang = PI - ((link_count - 1) as f32 * FRAC_PI_8) + (link_index as f32 * FRAC_PI_4);
        let pin_radius = self.hovered_pin_radius(pin_pos, mouse_pos);

        pos2(
            pin_pos.x + f32::cos(ang) * pin_radius,
            pin_pos.y - f32::sin(ang) * pin_radius,
        )
    }

    pub(crate) fn format_node(&self, node: &mut Node) {
        node.color_style.background = self.colors[ColorStyle::NodeBackground as usize];
        node.color_style.background_hovered =
            self.colors[ColorStyle::NodeBackgroundHovered as usize];
        node.color_style.background_selected =
            self.colors[ColorStyle::NodeBackgroundSelected as usize];
        node.color_style.header = self.colors[ColorStyle::NodeHeader as usize];
        node.color_style.header_hovered = self.colors[ColorStyle::NodeHeaderHovered as usize];
        node.color_style.header_selected = self.colors[ColorStyle::NodeHeaderSelected as usize];
        node.layout_style.corner_rounding = self.node_corner_rounding;
        node.layout_style.padding =
            Vec2::new(self.node_padding_horizontal, self.node_padding_vertical);
        node.layout_style.border_thickness = self.node_border_thickness;
    }

    pub(crate) fn format_pin(&self, pin: &mut PinData, args: PinArgs) {
        pin.shape = args.shape;
        pin.flags = args.flags.unwrap_or(0);
        pin.color_style.background = args
            .background
            .unwrap_or(self.colors[ColorStyle::Pin as usize]);
        pin.color_style.hovered = args
            .hovered
            .unwrap_or(self.colors[ColorStyle::PinHovered as usize]);
    }

    pub(crate) fn format_link(&self, link: &mut LinkData, args: LinkArgs) {
        link.color_style.base = args.base.unwrap_or(self.colors[ColorStyle::Link as usize]);
        link.color_style.hovered = args
            .hovered
            .unwrap_or(self.colors[ColorStyle::LinkHovered as usize]);
        link.color_style.selected = args
            .selected
            .unwrap_or(self.colors[ColorStyle::LinkSelected as usize]);
    }
}
