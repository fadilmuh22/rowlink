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

fn startup_worker() -> impl iced::futures::Stream<Item = Message> {
    stream::channel(1, async |mut output| {
        let _ = output.send(Message::Startup).await;
    })
}

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
        Subscription::run(startup_worker),
        Subscription::run(signal_worker),
        iced::event::listen().map(Message::IcedEvent),
    ])
}

// --- Update & View ---
fn update(state: &mut Rowlink, message: Message) -> iced::Task<Message> {
    match message {
        Message::Startup => iced::Task::done(Message::SetInputRegion {
            id: state.current_id.unwrap_or(IcedId::unique()),
            callback: ActionCallback::new(|_region| {}),
        }),
        Message::SignalReceived => {
            state.visible = true;
            state.input_buffer.clear();
            let settings = NewLayerShellSettings {
                size: None,
                anchor: Anchor::all(),
                layer: Layer::Overlay,
                exclusive_zone: Some(-1),
                events_transparent: true,
                keyboard_interactivity: KeyboardInteractivity::OnDemand, // Grab keyboard!
                ..Default::default()
            };

            let (new_id, spawn_task) = Message::layershell_open(settings);

            let old_id = state.current_id.unwrap_or(IcedId::unique());
            state.current_id = Some(new_id);
            iced::Task::batch(vec![
                iced::Task::done(Message::RemoveWindow(old_id)),
                spawn_task,
            ])
        }

        Message::IcedEvent(Event::Keyboard(keyboard::Event::KeyPressed { key, .. })) => {
            println!("Key pressed: {:?}", key);
            match key {
                // ESCAPE Logic: Close the overlay
                keyboard::Key::Named(keyboard::key::Named::Escape) => {
                    state.visible = false;
                    state.input_buffer.clear();

                    let settings = NewLayerShellSettings {
                        anchor: Anchor::all(),
                        layer: Layer::Background,
                        keyboard_interactivity: KeyboardInteractivity::None,
                        events_transparent: true,
                        ..Default::default()
                    };

                    let (new_id, spawn_task) = Message::layershell_open(settings);
                    let old_id = state.current_id.unwrap();
                    state.current_id = Some(new_id);

                    iced::Task::batch(vec![
                        iced::Task::done(Message::RemoveWindow(old_id)),
                        spawn_task,
                    ])
                }
                keyboard::Key::Character(c) => {
                    let c_char = c.chars().next().unwrap().to_ascii_uppercase();

                    if state.zoomed_cell.is_none() {
                        if c_char.is_ascii_uppercase() {
                            state.input_buffer.push(c_char);
                        }

                        if state.input_buffer.len() >= 2 {
                            let chars: Vec<char> = state.input_buffer.chars().collect();
                            let row = (chars[0] as u32 - 'A' as u32) as i32;
                            let col = (chars[1] as u32 - 'A' as u32) as i32;

                            state.zoomed_cell = Some((row, col));
                            state.input_buffer.clear();
                            state.grid_cache.clear();
                        }
                        iced::Task::none()
                    }
                    // Step 2: Handle 8x3 Precision Zoom
                    else {
                        // Map the physical key to (sub_row, sub_col)
                        let sub_coords = match c.as_str() {
                            // Row 1
                            "q" | "Q" => Some((0, 0)),
                            "w" | "W" => Some((0, 1)),
                            "e" | "E" => Some((0, 2)),
                            "r" | "R" => Some((0, 3)),
                            "u" | "U" => Some((0, 4)),
                            "i" | "I" => Some((0, 5)),
                            "o" | "O" => Some((0, 6)),
                            "p" | "P" => Some((0, 7)),
                            // Row 2
                            "a" | "A" => Some((1, 0)),
                            "s" | "S" => Some((1, 1)),
                            "d" | "D" => Some((1, 2)),
                            "f" | "F" => Some((1, 3)),
                            "j" | "J" => Some((1, 4)),
                            "k" | "K" => Some((1, 5)),
                            "l" | "L" => Some((1, 6)),
                            ";" => Some((1, 7)),
                            // Row 3
                            "z" | "Z" => Some((2, 0)),
                            "x" | "X" => Some((2, 1)),
                            "c" | "C" => Some((2, 2)),
                            "v" | "V" => Some((2, 3)),
                            "n" | "N" => Some((2, 4)),
                            "m" | "M" => Some((2, 5)),
                            "," => Some((2, 6)),
                            "." => Some((2, 7)),
                            _ => None,
                        };

                        if let Some((sub_row, sub_col)) = sub_coords {
                            let (main_row, main_col) = state.zoomed_cell.unwrap();

                            // Reset state
                            state.visible = false;
                            state.input_buffer.clear();
                            state.zoomed_cell = None;
                            state.grid_cache.clear();

                            // Prepare Ghost Window settings
                            let settings = NewLayerShellSettings {
                                anchor: Anchor::all(),
                                layer: Layer::Background,
                                keyboard_interactivity: KeyboardInteractivity::None,
                                events_transparent: true,
                                ..Default::default()
                            };

                            let (new_id, spawn_task) = Message::layershell_open(settings);
                            let old_id = state.current_id.unwrap();
                            state.current_id = Some(new_id);

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
                keyboard::Key::Named(keyboard::key::Named::Space) => {
                    // Grab the currently zoomed cell (could be None or Some((row, col)))
                    let target_cell = state.zoomed_cell;

                    // Reset all state
                    state.visible = false;
                    state.input_buffer.clear();
                    state.zoomed_cell = None;
                    state.grid_cache.clear();

                    // Swap back to the GHOST window BEFORE clicking
                    let settings = NewLayerShellSettings {
                        anchor: Anchor::all(),
                        layer: Layer::Background,
                        keyboard_interactivity: KeyboardInteractivity::None,
                        events_transparent: true,
                        ..Default::default()
                    };

                    let (new_id, spawn_task) = Message::layershell_open(settings);
                    let old_id = state.current_id.unwrap();
                    state.current_id = Some(new_id);

                    return iced::Task::batch(vec![
                        iced::Task::done(Message::RemoveWindow(old_id)),
                        iced::Task::done(Message::ExecuteMoveCenter(target_cell)),
                        spawn_task,
                    ]);
                }
                _ => iced::Task::none(),
            }
        }

        Message::ExecuteMovePrecision(main_row, main_col, sub_row, sub_col) => {
            let screen_w = 1920.0;
            let screen_h = 1080.0;

            let cell_w = screen_w / 26.0;
            let cell_h = screen_h / 26.0;

            // Main Cell Start Coordinates
            let main_x = main_col as f32 * cell_w;
            let main_y = main_row as f32 * cell_h;

            let padding = 4.0;
            let sub_container_w = cell_w - (padding * 2.0);
            let sub_container_h = cell_h - (padding * 2.0);

            // Sub Cell Dimensions for 8 columns x 3 rows
            let sub_w = sub_container_w / 8.0;
            let sub_h = sub_container_h / 3.0;

            // Center of the target sub-cell
            let target_x = main_x + (sub_col as f32 * sub_w) + (sub_w / 2.0);
            let target_y = main_y + (sub_row as f32 * sub_h) + (sub_h / 2.0);

            let final_x = target_x.round() as i32;
            let final_y = target_y.round() as i32;

            // Zero out and move (Enigo Hack)
            let _ = state.enigo.move_mouse(-10000, -10000, Coordinate::Rel);
            std::thread::sleep(std::time::Duration::from_millis(5));
            let _ = state.enigo.move_mouse(final_x, final_y, Coordinate::Rel);
            std::thread::sleep(std::time::Duration::from_millis(15));
            let _ = state.enigo.button(Button::Left, Direction::Click);

            iced::Task::none()
        }
        Message::ExecuteMoveCenter(target_cell) => {
            let screen_w = 1920.0;
            let screen_h = 1080.0;

            let (target_x, target_y) = match target_cell {
                Some((r, c)) => {
                    let cell_w = screen_w / 26.0;
                    let cell_h = screen_h / 26.0;
                    (
                        (c as f32 * cell_w) + (cell_w / 2.0),
                        (r as f32 * cell_h) + (cell_h / 2.0),
                    )
                }
                None => (screen_w / 2.0, screen_h / 2.0),
            };

            // --- CRITICAL: Wait for window to vanish ---
            std::thread::sleep(std::time::Duration::from_millis(60));

            let _ = state.enigo.move_mouse(-10000, -10000, Coordinate::Rel);
            std::thread::sleep(std::time::Duration::from_millis(5));
            let _ = state.enigo.move_mouse(
                target_x.round() as i32,
                target_y.round() as i32,
                Coordinate::Rel,
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
            let _ = state.enigo.button(Button::Left, Direction::Click);

            iced::Task::none()
        }
        _ => iced::Task::none(),
    }
}

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
            let cell_width = bounds.width / 26.0;
            let cell_height = bounds.height / 26.0;

            // Define the border style
            let border_stroke = canvas::Stroke {
                style: Style::Solid(Color::from_rgba(1.0, 1.0, 1.0, 0.15)),
                width: 1.0,
                ..Default::default()
            };

            if let Some((zoom_r, zoom_c)) = self.zoomed_cell {
                let main_x = zoom_c as f32 * cell_width;
                let main_y = zoom_r as f32 * cell_height;

                // Padding ensures the subgrid feels "nested" and neat
                let padding = 4.0;
                let sub_container_w = cell_width - (padding * 2.0);
                let sub_container_h = cell_height - (padding * 2.0);

                // We removed the frame.fill_rectangle to keep it 100% transparent.
                // If you find it hard to see, you can add a tiny glow/border instead.

                let sub_w = sub_container_w / 8.0;
                let sub_h = sub_container_h / 3.0;

                let labels = [
                    ["Q", "W", "E", "R", "U", "I", "O", "P"],
                    ["A", "S", "D", "F", "J", "K", "L", ";"],
                    ["Z", "X", "C", "V", "N", "M", ",", "."],
                ];

                for r in 0..3 {
                    for c in 0..8 {
                        let x = main_x + padding + (c as f32 * sub_w);
                        let y = main_y + padding + (r as f32 * sub_h);

                        // Use high contrast for transparent backgrounds
                        let text_color = if r == 1 {
                            Color::from_rgb(0.0, 1.0, 0.5) // Vibrant Green for Home Row
                        } else {
                            Color::from_rgba(1.0, 1.0, 1.0, 0.8) // Soft White for others
                        };

                        // Draw Sub-Cell Text
                        frame.fill_text(Text {
                            content: labels[r][c].to_string(),
                            position: Point::new(x + sub_w / 2.0, y + sub_h / 2.0),
                            color: text_color,
                            size: 11.0.into(),
                            align_x: iced::widget::text::Alignment::Center,
                            align_y: iced::alignment::Vertical::Center,
                            font: Font::MONOSPACE,
                            ..Default::default()
                        });
                    }
                }
            } else {
                // --- DRAW MAIN 26x26 GRID ---
                for r in 0..26 {
                    for c in 0..26 {
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
                                (b'A' + r as u8) as char,
                                (b'A' + c as u8) as char
                            ),
                            position: Point::new(x + cell_width / 2.0, y + cell_height / 2.0),
                            color: Color::from_rgb(1.0, 0.8, 0.2), // Yellow
                            size: 11.0.into(),
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
