use enigo::{Button, Coordinate, Direction, Enigo, InputResult, Mouse, Settings as EnigoSettings};
use iced::futures::sink::SinkExt;
use iced::widget::canvas::{self, Canvas, Style, Text};
use iced::{
    Color, Element, Event, Fill, Font, Point, Rectangle, Renderer, Subscription, Theme, keyboard,
    stream,
};
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
    delay_double_click_ms: u64,
    scroll_lines: i32,
    scroll_page_lines: i32,
    scroll_natural: bool,
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

impl AppConfig {
    fn get_main_cell_size(&self, width: f32, height: f32) -> (f32, f32) {
        (
            width / self.main_grid_size,
            height / self.main_grid_size,
        )
    }

    fn get_main_cell_center(&self, width: f32, height: f32, row: i32, col: i32) -> (f32, f32) {
        let (w, h) = self.get_main_cell_size(width, height);
        (
            (col as f32 * w) + (w / HALF),
            (row as f32 * h) + (h / HALF),
        )
    }

    fn get_precision_target(
        &self,
        width: f32,
        height: f32,
        main_row: i32,
        main_col: i32,
        sub_row: i32,
        sub_col: i32,
    ) -> (f32, f32) {
        let (cell_w, cell_h) = self.get_main_cell_size(width, height);
        let main_x = main_col as f32 * cell_w;
        let main_y = main_row as f32 * cell_h;

        let sub_container_w = cell_w - (self.sub_padding * DOUBLE);
        let sub_container_h = cell_h - (self.sub_padding * DOUBLE);

        let sub_w = sub_container_w / self.sub_cols as f32;
        let sub_h = sub_container_h / self.sub_rows as f32;

        let target_x = main_x + self.sub_padding + (sub_col as f32 * sub_w) + (sub_w / HALF);
        let target_y = main_y + self.sub_padding + (sub_row as f32 * sub_h) + (sub_h / HALF);

        (target_x, target_y)
    }
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
            delay_double_click_ms: 120,
            scroll_lines: 1,
            scroll_page_lines: 10,
            scroll_natural: true,
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
        if config_path.exists()
            && let Ok(file) = std::fs::File::open(config_path)
            && let Ok(cfg) = serde_yaml::from_reader(file)
        {
            println!("Loaded config from file.");
            return cfg;
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
    enigo: Option<Enigo>,
    visible: bool,
    grid_cache: canvas::Cache,
    current_id: Option<IcedId>,
    zoomed_cell: Option<(i32, i32)>,
    last_mouse_pos: Option<(f32, f32)>,
}

impl Rowlink {
    fn perform_enigo_action<F>(&mut self, mut action: F)
    where
        F: FnMut(&mut Enigo) -> InputResult<()>,
    {
        if self.enigo.is_none() {
            self.enigo = Enigo::new(&EnigoSettings::default()).ok();
        }

        if let Some(enigo) = self.enigo.as_mut()
            && let Err(e) = action(enigo)
        {
            eprintln!("Enigo Error (will retry): {:?}", e);

            self.enigo = Enigo::new(&EnigoSettings::default()).ok();

            if let Some(enigo_retry) = self.enigo.as_mut() {
                let _ = action(enigo_retry);
            }
        }
    }
}

impl Default for Rowlink {
    fn default() -> Self {
        Self {
            input_buffer: String::new(),
            enigo: Some(Enigo::new(&EnigoSettings::default()).expect("Enigo init failed")),
            visible: false,
            grid_cache: canvas::Cache::default(),
            current_id: None,
            zoomed_cell: None,
            last_mouse_pos: None,
        }
    }
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
enum Message {
    Startup,
    SignalReceived,
    ExecuteMovePrecision(i32, i32, i32, i32, bool),
    ExecuteMoveCenter(Option<(i32, i32)>, bool),
    ExecuteScroll(Option<(i32, i32)>, i32, i32),
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

fn move_sequence(enigo: &mut Enigo, x: f32, y: f32) -> InputResult<()> {
    std::thread::sleep(std::time::Duration::from_millis(
        cfg().delay_surface_destroy_ms,
    ));
    enigo.move_mouse(-10000, -10000, Coordinate::Rel)?;
    std::thread::sleep(std::time::Duration::from_millis(
        cfg().delay_wayland_zero_ms,
    ));
    enigo.move_mouse(x.round() as i32, y.round() as i32, Coordinate::Rel)?;
    std::thread::sleep(std::time::Duration::from_millis(
        cfg().delay_wayland_move_ms,
    ));
    Ok(())
}

fn click_sequence(
    enigo: &mut Enigo,
    x: f32,
    y: f32,
    is_double: bool,
    same_pos: bool,
) -> InputResult<()> {
    if !same_pos {
        move_sequence(enigo, x, y)?;
    }

    enigo.button(Button::Left, Direction::Click)?;
    if is_double {
        std::thread::sleep(std::time::Duration::from_millis(
            cfg().delay_double_click_ms,
        ));
        enigo.button(Button::Left, Direction::Click)?;
    }
    Ok(())
}

fn scroll_sequence(
    enigo: &mut Enigo,
    x: f32,
    y: f32,
    dx: i32,
    dy: i32,
    same_pos: bool,
) -> InputResult<()> {
    if !same_pos {
        move_sequence(enigo, x, y)?;
    }

    let (final_dx, final_dy) = if cfg().scroll_natural {
        (-dx, -dy)
    } else {
        (dx, dy)
    };

    if final_dx != 0 {
        enigo.scroll(final_dx, enigo::Axis::Horizontal)?;
    }
    if final_dy != 0 {
        enigo.scroll(final_dy, enigo::Axis::Vertical)?;
    }
    Ok(())
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
            state.last_mouse_pos = None;
            let (new_id, spawn_task) = Message::layershell_open(get_layer_settings(true));
            let old_id = state.current_id.replace(new_id).unwrap_or(IcedId::unique());
            iced::Task::batch(vec![
                iced::Task::done(Message::RemoveWindow(old_id)),
                spawn_task,
            ])
        }
        Message::IcedEvent(Event::Keyboard(keyboard::Event::KeyPressed {
            key, modifiers, ..
        })) => {
            match key {
                keyboard::Key::Named(keyboard::key::Named::Escape) => {
                    if !state.input_buffer.is_empty() {
                        state.input_buffer.pop();
                        state.grid_cache.clear();
                        iced::Task::none()
                    } else if state.zoomed_cell.is_some() {
                        state.zoomed_cell = None;
                        state.grid_cache.clear();
                        iced::Task::none()
                    } else {
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
                    let is_double = modifiers.shift();

                    state.visible = false;
                    state.input_buffer.clear();
                    state.zoomed_cell = None;
                    state.grid_cache.clear();
                    let (new_id, spawn_task) = Message::layershell_open(get_layer_settings(false));
                    let old_id = state.current_id.replace(new_id).unwrap();
                    iced::Task::batch(vec![
                        iced::Task::done(Message::RemoveWindow(old_id)),
                        iced::Task::done(Message::ExecuteMoveCenter(target_cell, is_double)),
                        spawn_task,
                    ])
                }
                keyboard::Key::Character(c) => {
                    if state.zoomed_cell.is_some() && modifiers.control() {
                        let step = cfg().scroll_lines;
                        let page_step = cfg().scroll_page_lines;

                        let (dx, dy) = match c.as_str() {
                            "j" => (0, -step),      // Down (Negative Y in Enigo usually implies down)
                            "k" => (0, step),       // Up
                            "h" => (-step, 0),      // Left
                            "l" => (step, 0),       // Right
                            "d" => (0, -page_step), // Page Down
                            "u" => (0, page_step),  // Page Up
                            _ => (0, 0),
                        };

                        if dx != 0 || dy != 0 {
                            return iced::Task::done(Message::ExecuteScroll(
                                state.zoomed_cell,
                                dx,
                                dy,
                            ));
                        }
                    }
                    let c_char = c.chars().next().unwrap();
                    if state.zoomed_cell.is_none() {
                        let c_upper = c_char.to_ascii_uppercase();
                        if c_upper.is_ascii_uppercase() {
                            state.input_buffer.push(c_upper);
                            state.grid_cache.clear();
                        }
                        if state.input_buffer.len() >= 2 {
                            let chars: Vec<char> = state.input_buffer.chars().collect();
                            let row = (chars[0] as u32 - BASE_CHAR) as i32;
                            let col = (chars[1] as u32 - BASE_CHAR) as i32;

                            if modifiers.shift() {
                                state.visible = false;
                                state.input_buffer.clear();
                                state.grid_cache.clear();
                                let (new_id, spawn_task) =
                                    Message::layershell_open(get_layer_settings(false));
                                let old_id = state.current_id.replace(new_id).unwrap();

                                return iced::Task::batch(vec![
                                    iced::Task::done(Message::RemoveWindow(old_id)),
                                    iced::Task::done(Message::ExecuteMoveCenter(
                                        Some((row, col)),
                                        true,
                                    )), // Double click
                                    spawn_task,
                                ]);
                            }

                            state.zoomed_cell = Some((row, col));
                            state.input_buffer.clear();
                            state.grid_cache.clear();
                        }
                        iced::Task::none()
                    } else if let Some((sub_row, sub_col)) = map_key_to_subgrid(c_char) {
                        let (main_row, main_col) = state.zoomed_cell.unwrap();
                        let is_double = modifiers.shift();

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
                                main_row, main_col, sub_row, sub_col, is_double,
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
        Message::ExecuteMovePrecision(main_row, main_col, sub_row, sub_col, is_double) => {
            let (target_x, target_y) = cfg().get_precision_target(
                cfg().screen_width,
                cfg().screen_height,
                main_row,
                main_col,
                sub_row,
                sub_col,
            );
            let same_pos = state.last_mouse_pos == Some((target_x, target_y));
            state.perform_enigo_action(|enigo| {
                click_sequence(enigo, target_x, target_y, is_double, same_pos)
            });
            state.last_mouse_pos = Some((target_x, target_y));
            iced::Task::none()
        }
        Message::ExecuteMoveCenter(target_cell, is_double) => {
            let (target_x, target_y) = match target_cell {
                Some((r, c)) => {
                    cfg().get_main_cell_center(cfg().screen_width, cfg().screen_height, r, c)
                }
                None => (cfg().screen_width / HALF, cfg().screen_height / HALF),
            };
            let same_pos = state.last_mouse_pos == Some((target_x, target_y));
            state.perform_enigo_action(|enigo| {
                click_sequence(enigo, target_x, target_y, is_double, same_pos)
            });
            state.last_mouse_pos = Some((target_x, target_y));
            iced::Task::none()
        }
        Message::ExecuteScroll(target_cell, dx, dy) => {
            let (target_x, target_y) = match target_cell {
                Some((r, c)) => {
                    cfg().get_main_cell_center(cfg().screen_width, cfg().screen_height, r, c)
                }
                None => (cfg().screen_width / HALF, cfg().screen_height / HALF),
            };
            let same_pos = state.last_mouse_pos == Some((target_x, target_y));
            state.perform_enigo_action(|enigo| {
                scroll_sequence(enigo, target_x, target_y, dx, dy, same_pos)
            });
            state.last_mouse_pos = Some((target_x, target_y));
            iced::Task::none()
        }
        _ => iced::Task::none(),
    }
}

// --- View & Style ---

fn view(state: &'_ Rowlink) -> Element<'_, Message> {
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
            let (cell_width, cell_height) =
                cfg().get_main_cell_size(bounds.width, bounds.height);
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
                for (r_idx, row_str) in cfg().sub_labels.iter().enumerate() {
                    if r_idx >= cfg().sub_rows as usize {
                        break;
                    }

                    for (c_idx, label_char) in row_str.chars().enumerate() {
                        if c_idx >= cfg().sub_cols as usize {
                            break;
                        }

                        let (target_x, target_y) = cfg().get_precision_target(
                            bounds.width,
                            bounds.height,
                            zoom_r,
                            zoom_c,
                            r_idx as i32,
                            c_idx as i32,
                        );

                        let text_color = if r_idx == 1 {
                            cfg().color_sub_home_row.to_iced()
                        } else {
                            cfg().color_sub_default.to_iced()
                        };

                        frame.fill_text(Text {
                            content: label_char.to_string(),
                            position: Point::new(target_x, target_y),
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
                        (stroke_normal, cfg().color_main_text.to_iced())
                    } else {
                        (stroke_dimmed, cfg().color_text_dimmed.to_iced())
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
                        let (center_x, center_y) =
                            cfg().get_main_cell_center(bounds.width, bounds.height, r, c);
                        frame.stroke(
                            &iced::widget::canvas::Path::rectangle(
                                Point::new(x, y),
                                iced::Size::new(cell_width, cell_height),
                            ),
                            current_stroke,
                        );
                        frame.fill_text(Text {
                            content: format!(
                                "{}{}",
                                (BASE_BYTE + r as u8) as char,
                                (BASE_BYTE + c as u8) as char
                            ),
                            position: Point::new(center_x, center_y),
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
