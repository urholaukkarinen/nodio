#![deny(clippy::all)]
use std::ops::Sub;
use std::sync::Arc;
use std::time::Duration;

use eframe::{egui, App, CreationContext, Frame, NativeOptions, Storage};
use egui::{pos2, Color32, FontData, FontDefinitions, FontFamily, RichText, Style, Widget};
use egui_toast::Toasts;
use indexmap::IndexMap;
use log::{debug, warn};
use parking_lot::RwLock;

use nodio_api::create_nodio_context;
use nodio_core::{Context, DeviceInfo, ProcessInfo, Uuid};
use nodio_core::{Node, NodeKind};
use nodio_gui_nodes::{AttributeFlags, Context as NodeContext, LinkArgs, PinArgs};
use slider::VolumeSlider;

use crate::egui::{Direction, Pos2, Response, Ui};

mod slider;

fn main() {
    pretty_env_logger::init();

    eframe::run_native(
        "Nodio",
        NativeOptions {
            ..Default::default()
        },
        Box::new(setup_app),
    );
}

fn setup_app(setup_ctx: &CreationContext) -> Box<dyn App> {
    let mut app = MyApp::default();

    let mut style = Style::default();
    style.visuals.override_text_color = Some(Color32::from_rgb(225, 225, 225));
    style.visuals.widgets.noninteractive.bg_fill = Color32::from_rgba_unmultiplied(50, 50, 50, 255);
    setup_ctx.egui_ctx.set_style(style);

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "custom".to_owned(),
        FontData::from_static(include_bytes!("../fonts/Lato-Regular.ttf")),
    );
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .unwrap()
        .insert(0, "custom".to_owned());
    fonts
        .families
        .get_mut(&FontFamily::Monospace)
        .unwrap()
        .push("custom".to_owned());

    setup_ctx.egui_ctx.set_fonts(fonts);

    if let Some(nodes_json) = setup_ctx
        .storage
        .and_then(|storage| storage.get_string("nodes"))
    {
        let mut ctx = app.ctx.write();
        for node in serde_json::from_str::<Vec<_>>(&nodes_json).unwrap_or_default() {
            ctx.add_node(node);
        }
    }

    if let Some(links_json) = setup_ctx
        .storage
        .and_then(|storage| storage.get_string("links"))
    {
        let mut ctx = app.ctx.write();
        for (id, start, end) in serde_json::from_str::<Vec<_>>(&links_json).unwrap_or_default() {
            if app.ui_links.insert(id, (start, end)).is_none() {
                ctx.connect_node(start, end).ok();
            }
        }
    }

    Box::new(app)
}

#[derive(Copy, Clone)]
enum ContextMenuKind {
    Node(Uuid),
    Editor,
}

struct MyApp {
    ctx: Arc<RwLock<dyn Context>>,
    node_ctx: NodeContext,
    /// Links between nodes (id, (start -> end))
    ui_links: IndexMap<Uuid, (Uuid, Uuid)>,
    context_menu_kind: Option<ContextMenuKind>,
    detached_link: Option<(Uuid, Uuid)>,

    should_save: bool,
}

impl Default for MyApp {
    fn default() -> Self {
        Self {
            ctx: create_nodio_context(),
            node_ctx: NodeContext::default(),
            ui_links: IndexMap::new(),
            context_menu_kind: None,
            detached_link: None,
            should_save: false,
        }
    }
}

impl MyApp {
    fn interact_and_draw(&mut self, ui_ctx: &egui::Context, ui: &mut Ui) {
        let node_count = self.ctx.read().nodes().len();

        let mut toasts = Toasts::new(ui_ctx)
            .anchor(
                ui_ctx
                    .available_rect()
                    .max
                    .sub(Pos2::new(10.0, 10.0))
                    .to_pos2(),
            )
            .align_to_end(true)
            .direction(Direction::BottomUp);

        self.node_ctx.begin_frame(ui);

        for node_idx in 0..node_count {
            let Node {
                id: node_id,
                kind: node_kind,
                volume: mut node_volume,
                active: node_active,
                present: node_present,
                peak_values: node_peak_values,
                display_name: node_display_name,
                pos: node_pos,
                ..
            } = self.ctx.read().nodes().get(node_idx).cloned().unwrap();

            let pin_args = match node_kind {
                NodeKind::Application | NodeKind::InputDevice => PinArgs::default(),
                NodeKind::OutputDevice => PinArgs {
                    flags: Some(AttributeFlags::EnableLinkDetachWithDragClick as _),
                    ..Default::default()
                },
            };

            let header_contents = |ui: &mut Ui| {
                ui.vertical_centered(|ui| {
                    ui.add_enabled_ui(node_present, move |ui| {
                        ui.label(format!(
                            "{}{}",
                            node_display_name,
                            if node_active { " 🔉" } else { "" }
                        ))
                    });
                });
            };

            let attr_contents = {
                let ctx = self.ctx.clone();
                move |ui: &mut Ui| {
                    ui.vertical(|ui| {
                        ui.add_enabled_ui(node_present, |ui| {
                            ui.spacing_mut().slider_width = 130.0;

                            if VolumeSlider::new(&mut node_volume, node_peak_values)
                                .ui(ui)
                                .changed()
                            {
                                ctx.write().set_volume(node_id, node_volume);
                            }
                        });
                    })
                    .response
                }
            };

            let mut node = self
                .node_ctx
                .add_node(node_id)
                .with_origin(pos2(node_pos.0, node_pos.1))
                .with_header(header_contents);

            match node_kind {
                NodeKind::Application | NodeKind::InputDevice => {
                    node.with_output_attribute(node_id, pin_args, attr_contents);
                }
                NodeKind::OutputDevice => {
                    node.with_input_attribute(node_id, pin_args, attr_contents);
                }
            }

            node.show(ui);
        }

        for (&id, &(start, end)) in self.ui_links.iter() {
            self.node_ctx
                .add_link(id, start, end, LinkArgs::default(), ui);
        }

        let nodes_response = self.node_ctx.end_frame(ui);

        self.context_menu(nodes_response);

        if let Some(id) = self.node_ctx.detached_link() {
            debug!("link detached: {}", id);

            if let Some((from, to)) = self.ui_links.remove(&id) {
                self.ctx.write().disconnect_node(from, to);
                self.detached_link = Some((from, to));
            }
        }

        if let Some(id) = self.node_ctx.dropped_link() {
            debug!("link dropped: {}", id);

            self.should_save = true;
            self.detached_link = None;
        }

        if let Some((start, end, from_snap)) = self.node_ctx.created_link() {
            debug!("link created: {}, ({} to {})", start, end, from_snap);

            match self.ctx.write().connect_node(start, end) {
                Ok(()) => {
                    self.ui_links.retain(|_, link| *link != (start, end));
                    self.ui_links.insert(Uuid::new_v4(), (start, end));
                }
                Err(err) => {
                    warn!("Failed to connect nodes: {}", err);

                    toasts.error(err.to_string(), Duration::from_secs(10));

                    if let Some((from, to)) = self.detached_link.take() {
                        self.ui_links.insert(Uuid::new_v4(), (from, to));
                    }
                }
            }

            self.should_save = true;
        }

        if node_count == 0 {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Right-click anywhere to add nodes")
                        .heading()
                        .color(ui.visuals().widgets.inactive.text_color()),
                );
            });
        }

        if ui.input().key_pressed(egui::Key::Delete) {
            self.remove_selected_nodes();
        }

        toasts.show();
    }

    fn context_menu(&mut self, nodes_response: Response) {
        let context_menu_kind = self
            .context_menu_kind
            .take()
            .or_else(|| self.node_ctx.hovered_node().map(ContextMenuKind::Node))
            .unwrap_or(ContextMenuKind::Editor);

        nodes_response.context_menu(|ui| {
            self.context_menu_kind = Some(context_menu_kind);

            match context_menu_kind {
                ContextMenuKind::Node(node_id) => self.node_context_menu_items(ui, node_id),
                ContextMenuKind::Editor => self.editor_context_menu_items(ui),
            }
        });
    }

    fn node_context_menu_items(&mut self, ui: &mut Ui, node_id: Uuid) {
        if ui.button("Remove").clicked() {
            self.ctx.write().remove_node(node_id);
            self.ui_links
                .retain(|_, (start, end)| *start != node_id && *end != node_id);

            // Remove other nodes too, when multiple nodes selected
            self.remove_selected_nodes();

            ui.close_menu();
        }
    }

    fn remove_selected_nodes(&mut self) {
        for &node_id in self.node_ctx.get_selected_nodes() {
            self.ctx.write().remove_node(node_id);
            self.ui_links
                .retain(|_, (start, end)| *start != node_id && *end != node_id);
        }
    }

    fn editor_context_menu_items(&mut self, ui: &mut Ui) {
        let mut added_node = None;

        let menu_pos = ui
            .add_enabled_ui(false, |ui| ui.label("Add node"))
            .response
            .rect
            .min;

        ui.menu_button("Application", |ui| {
            for process in self.ctx.read().application_processes() {
                Self::application_node_button(&mut added_node, menu_pos, ui, process);
            }
        });

        ui.menu_button("Input device", |ui| {
            for device in self.ctx.read().input_devices() {
                Self::device_node_button(
                    &mut added_node,
                    menu_pos,
                    ui,
                    device,
                    NodeKind::InputDevice,
                );
            }
        });

        ui.menu_button("Output device", |ui| {
            for device in self.ctx.read().output_devices() {
                Self::device_node_button(
                    &mut added_node,
                    menu_pos,
                    ui,
                    device,
                    NodeKind::OutputDevice,
                );
            }
        });

        if let Some(node) = added_node {
            self.ctx.write().add_node(node);
            self.should_save = true;
        }
    }

    fn application_node_button(
        added_node: &mut Option<Node>,
        menu_pos: Pos2,
        ui: &mut Ui,
        process: ProcessInfo,
    ) {
        if egui::Button::new(&process.display_name)
            .wrap(false)
            .ui(ui)
            .clicked()
        {
            added_node.replace(Node {
                kind: NodeKind::Application,
                display_name: process.display_name,
                filename: process.filename,
                pos: (menu_pos.x, menu_pos.y),
                process_id: Some(process.pid),
                ..Default::default()
            });
            ui.close_menu();
        }
    }

    fn device_node_button(
        added_node: &mut Option<Node>,
        menu_pos: Pos2,
        ui: &mut Ui,
        device: DeviceInfo,
        node_kind: NodeKind,
    ) {
        if egui::Button::new(&device.name).wrap(false).ui(ui).clicked() {
            added_node.replace(Node {
                id: device.id,
                kind: node_kind,
                display_name: device.name,
                pos: (menu_pos.x, menu_pos.y),
                ..Default::default()
            });
            ui.close_menu();
        }
    }
}

impl App for MyApp {
    fn update(&mut self, ui_ctx: &egui::Context, _frame: &mut Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ui_ctx, |ui| self.interact_and_draw(ui_ctx, ui));

        ui_ctx.request_repaint();
    }

    fn save(&mut self, storage: &mut dyn Storage) {
        debug!("Saving state");

        self.should_save = false;

        let mut nodes = self.ctx.read().nodes().to_vec();
        for node in nodes.iter_mut() {
            if let Some(pos) = self.node_ctx.node_pos(node.id) {
                node.pos = (pos.x, pos.y);
            }
        }

        let links: Vec<(Uuid, Uuid, Uuid)> = self
            .ui_links
            .iter()
            .map(|(id, (start, end))| (*id, *start, *end))
            .collect::<_>();

        storage.set_string("nodes", serde_json::to_string_pretty(&nodes).unwrap());
        storage.set_string("links", serde_json::to_string_pretty(&links).unwrap());
    }

    fn auto_save_interval(&self) -> Duration {
        if self.should_save {
            Duration::from_secs(0)
        } else {
            Duration::from_secs(30)
        }
    }
}
