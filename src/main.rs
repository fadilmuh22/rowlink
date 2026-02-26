use enigo::{Button, Coordinate, Direction, Enigo, Mouse, Settings as EnigoSettings};
use iced::futures::sink::SinkExt;
use iced::keyboard;
use iced::stream;
use iced::widget::canvas::{self, Canvas, Style, Text};
use iced::{Color, Element, Event, Fill, Font, Point, Rectangle, Renderer, Subscription, Theme};
use iced_layershell::actions::ActionCallback;
use iced_layershell::reexport::{
    Anchor, IcedId, KeyboardInteractivity, Layer, NewLayerShellSettings,
};
use iced_layershell::settings::{LayerShellSettings, Settings};
use iced_layershell::{application, to_layer_message};
use serde::Deserialize;
use std::sync::OnceLock;
use tokio::signal::unix::{SignalKind, signal};

// --- Global Config Singleton ---
static CONFIG: OnceLock<AppConfig> = OnceLock::new();

#[derive(Debug, Deserialize, Clone, Copy)]
struct ConfigColor {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl ConfigColor {
    fn to_iced(self) -> Color {
        Color {
            r: self.r,
            g: self.g,
            b: self.b,
            a: self.a,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
struct AppConfig {
    screen_width: f32,
    screen_height: f32,
    main_grid_size: f32,
    sub_rows: i32,
    sub_cols: i32,
    sub_padding: f32,
    font_size: f32,
    delay_surface_destroy_ms: u64,
    delay_wayland_zero_ms: u64,
    delay_wayland_move_ms: u64,
    // Colors
    color_grid_border: ConfigColor,
    color_main_text: ConfigColor,
    color_sub_home_row: ConfigColor,
    color_sub_default: ConfigColor,
    color_row_highlight: ConfigColor,
    color_text_dimmed: ConfigColor,
    color_border_dimmed: ConfigColor,
    // Labels (Dynamic 2D Grid)
    sub_labels: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            screen_width: 1920.0,
            screen_height: 1080.0,
            main_grid_size: 26.0,
            sub_rows: 3,
            sub_cols: 8,
            sub_padding: 4.0,
            font_size: 11.0,
            delay_surface_destroy_ms: 60,
            delay_wayland_zero_ms: 5,
            delay_wayland_move_ms: 20,
            color_grid_border: ConfigColor {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.15,
            },
            color_main_text: ConfigColor {
                r: 1.0,
                g: 0.8,
                b: 0.2,
                a: 1.0,
            },
            color_sub_home_row: ConfigColor {
                r: 0.0,
                g: 1.0,
                b: 0.5,
                a: 1.0,
            },
            color_sub_default: ConfigColor {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.8,
            },
            color_row_highlight: ConfigColor {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.1,
            },
            color_text_dimmed: ConfigColor {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.05,
            },
            color_border_dimmed: ConfigColor {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.02,
            },
            // Default QWERTY 8x3
            sub_labels: vec![
                "QWERUIOP".to_string(),
                "ASDFJKL;".to_string(),
                "ZXCVNM,.".to_string(),
            ],
        }
    }
}

// Math Constants
const HALF: f32 = 2.0;
const DOUBLE: f32 = 2.0;
const BASE_CHAR: u32 = 'A' as u32;
const BASE_BYTE: u8 = b'A';

// --- Config Loader ---
fn load_config() -> AppConfig {
    if let Some(proj_dirs) = directories::ProjectDirs::from("com", "rowlink", "rowlink") {
        let config_path = proj_dirs.config_dir().join("config.yaml");
        if config_path.exists() {
            if let Ok(file) = std::fs::File::open(config_path) {
                if let Ok(cfg) = serde_yaml::from_reader(file) {
                    println!("Loaded config from file.");
                    return cfg;
                }
            }
        }
    }
    println!("Using default config.");
    AppConfig::default()
}

fn cfg() -> &'static AppConfig {
    CONFIG.get_or_init(load_config)
}

pub fn main() -> Result<(), iced_layershell::Error> {
    let _ = cfg();
    application(Rowlink::default, namespace, update, view)
        .subscription(subscription)
        .style(style)
        .settings(Settings {
            id: Some(namespace()),
            layer_settings: LayerShellSettings {
                size: Some((1, 1)),
                anchor: Anchor::Top | Anchor::Left,
                layer: Layer::Background,
                keyboard_interactivity: KeyboardInteractivity::None,
                events_transparent: true,
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}

fn namespace() -> String {
    String::from("rowlink")
}

struct Rowlink {
    input_buffer: String,
    visible: bool,
    grid_cache: canvas::Cache,
    current_id: Option<IcedId>,
    zoomed_cell: Option<(i32, i32)>,
}

impl Default for Rowlink {
    fn default() -> Self {
        Self {
            input_buffer: String::new(),
            visible: false,
            grid_cache: canvas::Cache::default(),
            current_id: None,
            zoomed_cell: None,
        }
    }
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
enum Message {
    Startup,
    SignalReceived,
    ExecuteMovePrecision(i32, i32, i32, i32),
    ExecuteMoveCenter(Option<(i32, i32)>),
    IcedEvent(Event),
}

fn get_layer_settings(interactive: bool) -> NewLayerShellSettings {
    if interactive {
        NewLayerShellSettings {
            size: None,
            anchor: Anchor::all(),
            layer: Layer::Overlay,
            exclusive_zone: Some(-1),
            events_transparent: true,
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            ..Default::default()
        }
    } else {
        NewLayerShellSettings {
            anchor: Anchor::all(),
            layer: Layer::Background,
            keyboard_interactivity: KeyboardInteractivity::None,
            events_transparent: true,
            ..Default::default()
        }
    }
}

fn map_key_to_subgrid(c: char) -> Option<(i32, i32)> {
    let input_char = c.to_ascii_uppercase();

    for (r_idx, row_str) in cfg().sub_labels.iter().enumerate() {
        for (c_idx, key_char) in row_str.chars().enumerate() {
            if key_char.to_ascii_uppercase() == input_char {
                return Some((r_idx as i32, c_idx as i32));
            }
        }
    }
    None
}

fn perform_wayland_click(x: f32, y: f32) {
    let mut enigo = match Enigo::new(&EnigoSettings::default()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Rowlink Error: Could not connect to input system: {}", e);
            return;
        }
    };

    std::thread::sleep(std::time::Duration::from_millis(
        cfg().delay_surface_destroy_ms,
    ));
    let _ = enigo.move_mouse(-10000, -10000, Coordinate::Rel);
    std::thread::sleep(std::time::Duration::from_millis(
        cfg().delay_wayland_zero_ms,
    ));
    let _ = enigo.move_mouse(x.round() as i32, y.round() as i32, Coordinate::Rel);
    std::thread::sleep(std::time::Duration::from_millis(
        cfg().delay_wayland_move_ms,
    ));
    let _ = enigo.button(Button::Left, Direction::Click);
}

// --- Subscription & Update ---

fn signal_worker() -> impl iced::futures::Stream<Item = Message> {
    stream::channel(10, async |mut output| {
        let mut sig = signal(SignalKind::user_defined1()).expect("Failed to setup signal listener");
        loop {
            sig.recv().await;
            let _ = output.send(Message::SignalReceived).await;
        }
    })
}

fn subscription(_state: &Rowlink) -> Subscription<Message> {
    Subscription::batch(vec![
        Subscription::run(signal_worker),
        iced::event::listen().map(Message::IcedEvent),
    ])
}

fn update(state: &mut Rowlink, message: Message) -> iced::Task<Message> {
    match message {
        Message::LayerChange { id, .. } | Message::NewLayerShell { id, .. } => {
            state.current_id = Some(id);
            iced::Task::none()
        }
        Message::Startup => iced::Task::done(Message::SetInputRegion {
            id: state.current_id.unwrap_or(IcedId::unique()),
            callback: ActionCallback::new(|_region| {}),
        }),
        Message::SignalReceived => {
            state.visible = true;
            state.input_buffer.clear();
            let (new_id, spawn_task) = Message::layershell_open(get_layer_settings(true));
            let old_id = state.current_id.replace(new_id).unwrap_or(IcedId::unique());
            iced::Task::batch(vec![
                iced::Task::done(Message::RemoveWindow(old_id)),
                spawn_task,
            ])
        }
        Message::IcedEvent(Event::Keyboard(keyboard::Event::KeyPressed { key, .. })) => {
            match key {
                keyboard::Key::Named(keyboard::key::Named::Escape) => {
                    if !state.input_buffer.is_empty() {
                        state.input_buffer.pop();
                        state.grid_cache.clear();
                        iced::Task::none()
                    }
                    else if state.zoomed_cell.is_some() {
                        state.zoomed_cell = None;
                        state.grid_cache.clear();
                        iced::Task::none()
                    }
                    else {
                        state.visible = false;
                        state.input_buffer.clear();

                        let (new_id, spawn_task) =
                            Message::layershell_open(get_layer_settings(false));
                        let old_id = state.current_id.replace(new_id).unwrap();

                        iced::Task::batch(vec![
                            iced::Task::done(Message::RemoveWindow(old_id)),
                            spawn_task,
                        ])
                    }
                }
                keyboard::Key::Named(keyboard::key::Named::Space) => {
                    let target_cell = state.zoomed_cell;
                    state.visible = false;
                    state.input_buffer.clear();
                    state.zoomed_cell = None;
                    state.grid_cache.clear();
                    let (new_id, spawn_task) = Message::layershell_open(get_layer_settings(false));
                    let old_id = state.current_id.replace(new_id).unwrap();
                    iced::Task::batch(vec![
                        iced::Task::done(Message::RemoveWindow(old_id)),
                        iced::Task::done(Message::ExecuteMoveCenter(target_cell)),
                        spawn_task,
                    ])
                }
                keyboard::Key::Character(c) => {
                    let c_char = c.chars().next().unwrap();
                    if state.zoomed_cell.is_none() {
                        let c_upper = c_char.to_ascii_uppercase();
                        if c_upper.is_ascii_uppercase() {
                            state.input_buffer.push(c_upper);
                            // Clear cache to trigger redraw for row highlight
                            state.grid_cache.clear();
                        }
                        if state.input_buffer.len() >= 2 {
                            let chars: Vec<char> = state.input_buffer.chars().collect();
                            state.zoomed_cell = Some((
                                (chars[0] as u32 - BASE_CHAR) as i32,
                                (chars[1] as u32 - BASE_CHAR) as i32,
                            ));
                            state.input_buffer.clear();
                            state.grid_cache.clear();
                        }
                        iced::Task::none()
                    } else if let Some((sub_row, sub_col)) = map_key_to_subgrid(c_char) {
                        let (main_row, main_col) = state.zoomed_cell.unwrap();
                        state.visible = false;
                        state.input_buffer.clear();
                        state.zoomed_cell = None;
                        state.grid_cache.clear();
                        let (new_id, spawn_task) =
                            Message::layershell_open(get_layer_settings(false));
                        let old_id = state.current_id.replace(new_id).unwrap();
                        iced::Task::batch(vec![
                            iced::Task::done(Message::RemoveWindow(old_id)),
                            iced::Task::done(Message::ExecuteMovePrecision(
                                main_row, main_col, sub_row, sub_col,
                            )),
                            spawn_task,
                        ])
                    } else {
                        iced::Task::none()
                    }
                }
                _ => iced::Task::none(),
            }
        }
        Message::ExecuteMovePrecision(main_row, main_col, sub_row, sub_col) => {
            let cell_w = cfg().screen_width / cfg().main_grid_size;
            let cell_h = cfg().screen_height / cfg().main_grid_size;
            let main_x = main_col as f32 * cell_w;
            let main_y = main_row as f32 * cell_h;
            let sub_container_w = cell_w - (cfg().sub_padding * DOUBLE);
            let sub_container_h = cell_h - (cfg().sub_padding * DOUBLE);
            let sub_w = sub_container_w / cfg().sub_cols as f32;
            let sub_h = sub_container_h / cfg().sub_rows as f32;
            let target_x = main_x + cfg().sub_padding + (sub_col as f32 * sub_w) + (sub_w / HALF);
            let target_y = main_y + cfg().sub_padding + (sub_row as f32 * sub_h) + (sub_h / HALF);
            perform_wayland_click(target_x, target_y);
            iced::Task::none()
        }
        Message::ExecuteMoveCenter(target_cell) => {
            let (target_x, target_y) = match target_cell {
                Some((r, c)) => {
                    let cell_w = cfg().screen_width / cfg().main_grid_size;
                    let cell_h = cfg().screen_height / cfg().main_grid_size;
                    (
                        (c as f32 * cell_w) + (cell_w / HALF),
                        (r as f32 * cell_h) + (cell_h / HALF),
                    )
                }
                None => (cfg().screen_width / HALF, cfg().screen_height / HALF),
            };
            perform_wayland_click(target_x, target_y);
            iced::Task::none()
        }
        _ => iced::Task::none(),
    }
}

// --- View & Style ---

fn view(state: &Rowlink) -> Element<Message> {
    if !state.visible {
        return iced::widget::container(iced::widget::space()).into();
    }
    Canvas::new(state).width(Fill).height(Fill).into()
}

fn style(_state: &Rowlink, _theme: &Theme) -> iced::theme::Style {
    iced::theme::Style {
        background_color: Color::TRANSPARENT,
        text_color: Color::WHITE,
    }
}

// --- Canvas Program ---

impl<Message> canvas::Program<Message> for Rowlink {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let grid = self.grid_cache.draw(renderer, bounds.size(), |frame| {
            let cell_width = bounds.width / cfg().main_grid_size;
            let cell_height = bounds.height / cfg().main_grid_size;
            let stroke_normal = canvas::Stroke {
                style: Style::Solid(cfg().color_grid_border.to_iced()),
                width: 1.0,
                ..Default::default()
            };

            let stroke_dimmed = canvas::Stroke {
                style: Style::Solid(cfg().color_border_dimmed.to_iced()),
                width: 1.0,
                ..Default::default()
            };

            if let Some((zoom_r, zoom_c)) = self.zoomed_cell {
                let main_x = zoom_c as f32 * cell_width;
                let main_y = zoom_r as f32 * cell_height;
                let sub_container_w = cell_width - (cfg().sub_padding * DOUBLE);
                let sub_container_h = cell_height - (cfg().sub_padding * DOUBLE);
                let sub_w = sub_container_w / cfg().sub_cols as f32;
                let sub_h = sub_container_h / cfg().sub_rows as f32;

                for (r_idx, row_str) in cfg().sub_labels.iter().enumerate() {
                    if r_idx >= cfg().sub_rows as usize {
                        break;
                    }

                    for (c_idx, label_char) in row_str.chars().enumerate() {
                        if c_idx >= cfg().sub_cols as usize {
                            break;
                        }

                        let x = main_x + cfg().sub_padding + (c_idx as f32 * sub_w);
                        let y = main_y + cfg().sub_padding + (r_idx as f32 * sub_h);

                        let text_color = if r_idx == 1 {
                            cfg().color_sub_home_row.to_iced()
                        } else {
                            cfg().color_sub_default.to_iced()
                        };

                        frame.fill_text(Text {
                            content: label_char.to_string(),
                            position: Point::new(x + sub_w / HALF, y + sub_h / HALF),
                            color: text_color,
                            size: cfg().font_size.into(),
                            align_x: iced::widget::text::Alignment::Center,
                            align_y: iced::alignment::Vertical::Center,
                            font: Font::MONOSPACE,
                            ..Default::default()
                        });
                    }
                }
            } else {
                let active_row = if self.input_buffer.len() == 1 {
                    let c = self.input_buffer.chars().next().unwrap();
                    Some((c as u32 - BASE_CHAR) as i32)
                } else {
                    None
                };

                for r in 0..cfg().main_grid_size as i32 {
                    let is_active_row = Some(r) == active_row;
                    let is_dimmed_mode = active_row.is_some();

                    let (current_stroke, current_text_color) = if !is_dimmed_mode || is_active_row {
                        (stroke_normal.clone(), cfg().color_main_text.to_iced())
                    } else {
                        (stroke_dimmed.clone(), cfg().color_text_dimmed.to_iced())
                    };

                    if is_active_row {
                        frame.fill_rectangle(
                            Point::new(0.0, r as f32 * cell_height),
                            iced::Size::new(bounds.width, cell_height),
                            cfg().color_row_highlight.to_iced(),
                        );
                    }

                    for c in 0..cfg().main_grid_size as i32 {
                        let x = c as f32 * cell_width;
                        let y = r as f32 * cell_height;
                        frame.stroke(
                            &iced::widget::canvas::Path::rectangle(
                                Point::new(x, y),
                                iced::Size::new(cell_width, cell_height),
                            ),
                            current_stroke.clone(),
                        );
                        frame.fill_text(Text {
                            content: format!(
                                "{}{}",
                                (BASE_BYTE + r as u8) as char,
                                (BASE_BYTE + c as u8) as char
                            ),
                            position: Point::new(x + cell_width / HALF, y + cell_height / HALF),
                            color: current_text_color,
                            size: cfg().font_size.into(),
                            align_x: iced::widget::text::Alignment::Center,
                            align_y: iced::alignment::Vertical::Center,
                            font: Font::MONOSPACE,
                            ..Default::default()
                        });
                    }
                }
            }
        });
        vec![grid]
    }
}
