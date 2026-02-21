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
use tokio::signal::unix::{SignalKind, signal};

// --- Configurable Constants ---
const SCREEN_W: f32 = 1920.0;
const SCREEN_H: f32 = 1080.0;
const MAIN_GRID_SIZE: f32 = 26.0;
const SUB_ROWS: i32 = 3;
const SUB_COLS: i32 = 8;
const SUB_PADDING: f32 = 4.0;
const FONT_SIZE: f32 = 11.0;

// Math / Layout Constants
const HALF: f32 = 2.0; // Divisor to find the exact center of a cell
const DOUBLE: f32 = 2.0; // Multiplier to account for padding on both sides
const BASE_CHAR: u32 = 'A' as u32;
const BASE_BYTE: u8 = b'A';

// Timings
const DELAY_SURFACE_DESTROY_MS: u64 = 60;
const DELAY_WAYLAND_ZERO_MS: u64 = 5;
const DELAY_WAYLAND_MOVE_MS: u64 = 20;

// Colors
const COLOR_GRID_BORDER: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.15,
};
const COLOR_MAIN_TEXT: Color = Color {
    r: 1.0,
    g: 0.8,
    b: 0.2,
    a: 1.0,
}; // Yellow
const COLOR_SUB_HOME_ROW: Color = Color {
    r: 0.0,
    g: 1.0,
    b: 0.5,
    a: 1.0,
}; // Vibrant Green
const COLOR_SUB_DEFAULT: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.8,
}; // Soft White

const SUB_LABELS: [[&str; 8]; 3] = [
    ["Q", "W", "E", "R", "U", "I", "O", "P"],
    ["A", "S", "D", "F", "J", "K", "L", ";"],
    ["Z", "X", "C", "V", "N", "M", ",", "."],
];

pub fn main() -> Result<(), iced_layershell::Error> {
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
    enigo: Enigo,
    visible: bool,
    grid_cache: canvas::Cache,
    current_id: Option<IcedId>,
    zoomed_cell: Option<(i32, i32)>,
}

impl Default for Rowlink {
    fn default() -> Self {
        Self {
            input_buffer: String::new(),
            enigo: Enigo::new(&EnigoSettings::default()).expect("Enigo init failed"),
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

// --- Helper Functions ---

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
    match c.to_ascii_uppercase() {
        'Q' => Some((0, 0)),
        'W' => Some((0, 1)),
        'E' => Some((0, 2)),
        'R' => Some((0, 3)),
        'U' => Some((0, 4)),
        'I' => Some((0, 5)),
        'O' => Some((0, 6)),
        'P' => Some((0, 7)),
        'A' => Some((1, 0)),
        'S' => Some((1, 1)),
        'D' => Some((1, 2)),
        'F' => Some((1, 3)),
        'J' => Some((1, 4)),
        'K' => Some((1, 5)),
        'L' => Some((1, 6)),
        ';' => Some((1, 7)),
        'Z' => Some((2, 0)),
        'X' => Some((2, 1)),
        'C' => Some((2, 2)),
        'V' => Some((2, 3)),
        'N' => Some((2, 4)),
        'M' => Some((2, 5)),
        ',' => Some((2, 6)),
        '.' => Some((2, 7)),
        _ => None,
    }
}

fn perform_wayland_click(enigo: &mut Enigo, x: f32, y: f32) {
    std::thread::sleep(std::time::Duration::from_millis(DELAY_SURFACE_DESTROY_MS));
    let _ = enigo.move_mouse(-10000, -10000, Coordinate::Rel);
    std::thread::sleep(std::time::Duration::from_millis(DELAY_WAYLAND_ZERO_MS));
    let _ = enigo.move_mouse(x.round() as i32, y.round() as i32, Coordinate::Rel);
    std::thread::sleep(std::time::Duration::from_millis(DELAY_WAYLAND_MOVE_MS));
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

        Message::IcedEvent(Event::Keyboard(keyboard::Event::KeyPressed { key, .. })) => match key {
            keyboard::Key::Named(keyboard::key::Named::Escape) => {
                state.visible = false;
                state.input_buffer.clear();

                let (new_id, spawn_task) = Message::layershell_open(get_layer_settings(false));
                let old_id = state.current_id.replace(new_id).unwrap();

                iced::Task::batch(vec![
                    iced::Task::done(Message::RemoveWindow(old_id)),
                    spawn_task,
                ])
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
                } else {
                    if let Some((sub_row, sub_col)) = map_key_to_subgrid(c_char) {
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
            }
            _ => iced::Task::none(),
        },

        Message::ExecuteMovePrecision(main_row, main_col, sub_row, sub_col) => {
            let cell_w = SCREEN_W / MAIN_GRID_SIZE;
            let cell_h = SCREEN_H / MAIN_GRID_SIZE;

            let main_x = main_col as f32 * cell_w;
            let main_y = main_row as f32 * cell_h;

            let sub_container_w = cell_w - (SUB_PADDING * DOUBLE);
            let sub_container_h = cell_h - (SUB_PADDING * DOUBLE);

            let sub_w = sub_container_w / SUB_COLS as f32;
            let sub_h = sub_container_h / SUB_ROWS as f32;

            let target_x = main_x + SUB_PADDING + (sub_col as f32 * sub_w) + (sub_w / HALF);
            let target_y = main_y + SUB_PADDING + (sub_row as f32 * sub_h) + (sub_h / HALF);

            perform_wayland_click(&mut state.enigo, target_x, target_y);
            iced::Task::none()
        }

        Message::ExecuteMoveCenter(target_cell) => {
            let (target_x, target_y) = match target_cell {
                Some((r, c)) => {
                    let cell_w = SCREEN_W / MAIN_GRID_SIZE;
                    let cell_h = SCREEN_H / MAIN_GRID_SIZE;
                    (
                        (c as f32 * cell_w) + (cell_w / HALF),
                        (r as f32 * cell_h) + (cell_h / HALF),
                    )
                }
                None => (SCREEN_W / HALF, SCREEN_H / HALF),
            };

            perform_wayland_click(&mut state.enigo, target_x, target_y);
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
            let cell_width = bounds.width / MAIN_GRID_SIZE;
            let cell_height = bounds.height / MAIN_GRID_SIZE;

            let border_stroke = canvas::Stroke {
                style: Style::Solid(COLOR_GRID_BORDER),
                width: 1.0,
                ..Default::default()
            };

            if let Some((zoom_r, zoom_c)) = self.zoomed_cell {
                let main_x = zoom_c as f32 * cell_width;
                let main_y = zoom_r as f32 * cell_height;

                let sub_container_w = cell_width - (SUB_PADDING * DOUBLE);
                let sub_container_h = cell_height - (SUB_PADDING * DOUBLE);

                let sub_w = sub_container_w / SUB_COLS as f32;
                let sub_h = sub_container_h / SUB_ROWS as f32;

                for r in 0..SUB_ROWS {
                    for c in 0..SUB_COLS {
                        let x = main_x + SUB_PADDING + (c as f32 * sub_w);
                        let y = main_y + SUB_PADDING + (r as f32 * sub_h);

                        let text_color = if r == 1 {
                            COLOR_SUB_HOME_ROW
                        } else {
                            COLOR_SUB_DEFAULT
                        };

                        frame.fill_text(Text {
                            content: SUB_LABELS[r as usize][c as usize].to_string(),
                            position: Point::new(x + sub_w / HALF, y + sub_h / HALF),
                            color: text_color,
                            size: FONT_SIZE.into(),
                            align_x: iced::widget::text::Alignment::Center,
                            align_y: iced::alignment::Vertical::Center,
                            font: Font::MONOSPACE,
                            ..Default::default()
                        });
                    }
                }
            } else {
                for r in 0..MAIN_GRID_SIZE as i32 {
                    for c in 0..MAIN_GRID_SIZE as i32 {
                        let x = c as f32 * cell_width;
                        let y = r as f32 * cell_height;

                        frame.stroke(
                            &iced::widget::canvas::Path::rectangle(
                                Point::new(x, y),
                                iced::Size::new(cell_width, cell_height),
                            ),
                            border_stroke.clone(),
                        );

                        frame.fill_text(Text {
                            content: format!(
                                "{}{}",
                                (BASE_BYTE + r as u8) as char,
                                (BASE_BYTE + c as u8) as char
                            ),
                            position: Point::new(x + cell_width / HALF, y + cell_height / HALF),
                            color: COLOR_MAIN_TEXT,
                            size: FONT_SIZE.into(),
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
