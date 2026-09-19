#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Layout, Margin, RichText, Stroke, Vec2,
};
use enigo::{Button, Direction, Enigo, Mouse, Settings};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey},
};
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const BG: Color32 = Color32::from_rgb(12, 13, 15);
const PANEL: Color32 = Color32::from_rgb(22, 23, 27);
const PANEL_2: Color32 = Color32::from_rgb(29, 30, 35);
const ORANGE: Color32 = Color32::from_rgb(255, 124, 36);
const ORANGE_SOFT: Color32 = Color32::from_rgb(255, 161, 82);
const ACCENT: Color32 = Color32::from_rgb(91, 214, 196);
const TEXT: Color32 = Color32::from_rgb(239, 240, 242);
const MUTED: Color32 = Color32::from_rgb(143, 146, 155);

#[derive(Clone, Copy, PartialEq)]
enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Middle => "Middle",
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Right => 1,
            Self::Middle => 2,
        }
    }
}

struct Shared {
    running: AtomicBool,
    shutdown: AtomicBool,
    interval_us: AtomicU64,
    button: AtomicU8,
    double_click: AtomicBool,
    finite: AtomicBool,
    target: AtomicU64,
    session_clicks: AtomicU64,
    total_clicks: AtomicU64,
    error_code: AtomicU32,
    wake: (Mutex<()>, Condvar),
}

impl Shared {
    fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            interval_us: AtomicU64::new(10_000),
            button: AtomicU8::new(0),
            double_click: AtomicBool::new(false),
            finite: AtomicBool::new(false),
            target: AtomicU64::new(100),
            session_clicks: AtomicU64::new(0),
            total_clicks: AtomicU64::new(0),
            error_code: AtomicU32::new(0),
            wake: (Mutex::new(()), Condvar::new()),
        }
    }
}

struct ClickEngine {
    shared: Arc<Shared>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ClickEngine {
    fn new() -> Self {
        let shared = Arc::new(Shared::new());
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("emberclick-engine".into())
            .spawn(move || click_worker(worker_shared))
            .expect("failed to start click engine");
        Self {
            shared,
            worker: Some(worker),
        }
    }

    fn start(&self) {
        self.shared.session_clicks.store(0, Ordering::Relaxed);
        self.shared.error_code.store(0, Ordering::Relaxed);
        self.shared.running.store(true, Ordering::Release);
        self.shared.wake.1.notify_one();
    }

    fn stop(&self) {
        self.shared.running.store(false, Ordering::Release);
        self.shared.wake.1.notify_one();
    }

    fn toggle(&self) {
        if self.is_running() {
            self.stop();
        } else {
            self.start();
        }
    }

    fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Acquire)
    }
}

impl Drop for ClickEngine {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        self.shared.running.store(false, Ordering::Release);
        self.shared.wake.1.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn click_worker(shared: Arc<Shared>) {
    let mut enigo = match Enigo::new(&Settings::default()) {
        Ok(enigo) => enigo,
        Err(_) => {
            shared.error_code.store(1, Ordering::Release);
            return;
        }
    };

    loop {
        if shared.shutdown.load(Ordering::Acquire) {
            return;
        }
        if !shared.running.load(Ordering::Acquire) {
            let guard = shared.wake.0.lock().unwrap_or_else(|e| e.into_inner());
            let _ = shared.wake.1.wait_timeout(guard, Duration::from_secs(1));
            continue;
        }

        let button = match shared.button.load(Ordering::Relaxed) {
            1 => Button::Right,
            2 => Button::Middle,
            _ => Button::Left,
        };
        let repetitions = if shared.double_click.load(Ordering::Relaxed) {
            2
        } else {
            1
        };

        for _ in 0..repetitions {
            if enigo.button(button, Direction::Click).is_err() {
                shared.error_code.store(2, Ordering::Release);
                shared.running.store(false, Ordering::Release);
                break;
            }
            shared.total_clicks.fetch_add(1, Ordering::Relaxed);
        }

        let completed = shared.session_clicks.fetch_add(1, Ordering::Relaxed) + 1;
        if shared.finite.load(Ordering::Relaxed)
            && completed >= shared.target.load(Ordering::Relaxed)
        {
            shared.running.store(false, Ordering::Release);
            continue;
        }

        let interval = Duration::from_micros(shared.interval_us.load(Ordering::Relaxed).max(1));
        let deadline = Instant::now() + interval;
        // Sleeping for most of the interval keeps CPU use low; a short yield phase
        // avoids the large scheduling jitter that makes fast clickers inconsistent.
        if interval > Duration::from_millis(2) {
            thread::sleep(interval.saturating_sub(Duration::from_micros(500)));
        }
        while shared.running.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::yield_now();
        }
    }
}

struct EmberClickApp {
    engine: ClickEngine,
    interval_ms: f64,
    mouse_button: MouseButton,
    double_click: bool,
    repeat_until_stopped: bool,
    repeat_count: u64,
    hotkey_id: Option<u32>,
    _hotkey_manager: Option<GlobalHotKeyManager>,
}

impl EmberClickApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_style(&cc.egui_ctx);
        let (manager, hotkey_id) = install_hotkey();
        Self {
            engine: ClickEngine::new(),
            interval_ms: 10.0,
            mouse_button: MouseButton::Left,
            double_click: false,
            repeat_until_stopped: true,
            repeat_count: 100,
            hotkey_id,
            _hotkey_manager: manager,
        }
    }

    fn sync_settings(&self) {
        self.engine.shared.interval_us.store(
            (self.interval_ms.clamp(0.1, 3_600_000.0) * 1_000.0) as u64,
            Ordering::Relaxed,
        );
        self.engine
            .shared
            .button
            .store(self.mouse_button.code(), Ordering::Relaxed);
        self.engine
            .shared
            .double_click
            .store(self.double_click, Ordering::Relaxed);
        self.engine
            .shared
            .finite
            .store(!self.repeat_until_stopped, Ordering::Relaxed);
        self.engine
            .shared
            .target
            .store(self.repeat_count.max(1), Ordering::Relaxed);
    }

    fn toggle(&self) {
        self.sync_settings();
        self.engine.toggle();
    }

    fn check_hotkey(&self) {
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if Some(event.id) == self.hotkey_id && event.state == HotKeyState::Pressed {
                self.toggle();
            }
        }
    }
}

impl eframe::App for EmberClickApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.check_hotkey();
        self.sync_settings();
        ctx.request_repaint_after(Duration::from_millis(33));
    }

    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.hotkey_id.is_none() && root_ui.input(|i| i.key_pressed(egui::Key::F6)) {
            self.toggle();
        }
        self.sync_settings();

        egui::Frame::new()
            .fill(BG)
            .inner_margin(Margin::same(24))
            .show(root_ui, |ui| {
                header(ui, self.engine.is_running());
                ui.add_space(22.0);

                ui.columns(2, |columns| {
                    columns[0].set_width(340.0);
                    card(&mut columns[0], "CLICK INTERVAL", |ui| {
                        ui.horizontal(|ui| {
                            let response = ui.add_sized(
                                [190.0, 48.0],
                                egui::DragValue::new(&mut self.interval_ms)
                                    .range(0.1..=3_600_000.0)
                                    .speed(0.5)
                                    .max_decimals(1)
                                    .suffix(" ms"),
                            );
                            response.on_hover_text("Delay between click actions");
                            ui.add_space(8.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new("RATE").size(10.0).color(MUTED));
                                let cps = 1000.0 / self.interval_ms.max(0.1);
                                ui.label(
                                    RichText::new(format!("{cps:.1} CPS"))
                                        .size(16.0)
                                        .color(ACCENT),
                                );
                            });
                        });
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            for (label, value) in [
                                ("1 ms", 1.0),
                                ("10 ms", 10.0),
                                ("50 ms", 50.0),
                                ("1 sec", 1000.0),
                            ] {
                                if chip(ui, label, (self.interval_ms - value).abs() < 0.01)
                                    .clicked()
                                {
                                    self.interval_ms = value;
                                }
                            }
                        });
                    });

                    columns[0].add_space(14.0);
                    card(&mut columns[0], "REPEAT", |ui| {
                        ui.horizontal(|ui| {
                            if choice(ui, "Until stopped", self.repeat_until_stopped).clicked() {
                                self.repeat_until_stopped = true;
                            }
                            if choice(ui, "Fixed amount", !self.repeat_until_stopped).clicked() {
                                self.repeat_until_stopped = false;
                            }
                        });
                        if !self.repeat_until_stopped {
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Actions").color(MUTED));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.add(
                                        egui::DragValue::new(&mut self.repeat_count)
                                            .range(1..=1_000_000_000_u64)
                                            .speed(1),
                                    );
                                });
                            });
                        }
                    });

                    card(&mut columns[1], "MOUSE BUTTON", |ui| {
                        ui.horizontal(|ui| {
                            for value in
                                [MouseButton::Left, MouseButton::Right, MouseButton::Middle]
                            {
                                if choice(ui, value.label(), self.mouse_button == value).clicked() {
                                    self.mouse_button = value;
                                }
                            }
                        });
                        ui.add_space(16.0);
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new("CLICK TYPE").size(10.0).color(MUTED));
                                ui.label(
                                    RichText::new(if self.double_click {
                                        "Double click"
                                    } else {
                                        "Single click"
                                    })
                                    .color(TEXT),
                                );
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.toggle_value(&mut self.double_click, "Double");
                            });
                        });
                    });

                    columns[1].add_space(14.0);
                    status_card(&mut columns[1], self);
                });

                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("F6  ·  START / STOP ANYWHERE")
                            .size(11.0)
                            .color(MUTED),
                    );
                    ui.add_space(10.0);
                    let running = self.engine.is_running();
                    let label = if running {
                        "STOP CLICKING"
                    } else {
                        "START CLICKING"
                    };
                    let button = egui::Button::new(
                        RichText::new(label)
                            .size(16.0)
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(if running {
                        Color32::from_rgb(190, 55, 43)
                    } else {
                        ORANGE
                    })
                    .stroke(Stroke::NONE)
                    .corner_radius(12)
                    .min_size(Vec2::new(ui.available_width().min(480.0), 54.0));
                    if ui.add(button).clicked() {
                        self.toggle();
                    }
                });
            });
    }
}

fn install_hotkey() -> (Option<GlobalHotKeyManager>, Option<u32>) {
    let Ok(manager) = GlobalHotKeyManager::new() else {
        return (None, None);
    };
    let hotkey = HotKey::new(None, Code::F6);
    let id = hotkey.id;
    if manager.register(hotkey).is_ok() {
        (Some(manager), Some(id))
    } else {
        (Some(manager), None)
    }
}

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = Color32::from_rgb(16, 17, 20);
    style.visuals.widgets.inactive.bg_fill = PANEL_2;
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(42, 43, 49);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, ORANGE_SOFT);
    style.visuals.widgets.active.bg_fill = ORANGE;
    style.visuals.selection.bg_fill = ORANGE;
    style.visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    style.spacing.item_spacing = Vec2::new(10.0, 10.0);
    style.spacing.button_padding = Vec2::new(14.0, 9.0);
    ctx.set_style_of(egui::Theme::Dark, style);
}

fn header(ui: &mut egui::Ui, running: bool) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(42.0, 42.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 11.0, ORANGE);
        ui.painter()
            .circle_filled(rect.center(), 7.0, Color32::WHITE);
        ui.painter().circle_filled(rect.center(), 3.0, ORANGE);
        ui.add_space(3.0);
        ui.vertical(|ui| {
            ui.label(
                RichText::new("EMBERCLICK")
                    .font(FontId::proportional(22.0))
                    .strong()
                    .color(TEXT),
            );
            ui.label(
                RichText::new("Fast. Precise. Lightweight.")
                    .size(11.0)
                    .color(MUTED),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let color = if running { ACCENT } else { MUTED };
            ui.label(
                RichText::new(if running { "ACTIVE" } else { "READY" })
                    .size(11.0)
                    .strong()
                    .color(color),
            );
            let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
            ui.painter().circle_filled(dot.center(), 4.0, color);
        });
    });
}

fn card(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, Color32::from_rgb(42, 43, 48)))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_min_width(300.0);
            ui.label(RichText::new(title).size(11.0).strong().color(ORANGE_SOFT));
            ui.add_space(12.0);
            add_contents(ui);
        });
}

fn status_card(ui: &mut egui::Ui, app: &EmberClickApp) {
    card(ui, "SESSION", |ui| {
        let count = app.engine.shared.session_clicks.load(Ordering::Relaxed);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Actions").size(11.0).color(MUTED));
                ui.label(
                    RichText::new(format_number(count))
                        .size(25.0)
                        .strong()
                        .color(TEXT),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let hotkey_text = if app.hotkey_id.is_some() {
                    "Global F6 ready"
                } else {
                    "F6 works in window"
                };
                ui.label(RichText::new(hotkey_text).size(11.0).color(ACCENT));
            });
        });
        let error = app.engine.shared.error_code.load(Ordering::Acquire);
        if error != 0 {
            ui.add_space(8.0);
            let message = if error == 1 {
                "Could not connect to the system input service."
            } else {
                "The operating system rejected simulated input."
            };
            ui.label(
                RichText::new(message)
                    .size(11.0)
                    .color(Color32::from_rgb(255, 105, 95)),
            );
        }
    });
}

fn choice(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).color(if selected {
            Color32::WHITE
        } else {
            MUTED
        }))
        .fill(if selected { ORANGE } else { PANEL_2 })
        .stroke(Stroke::new(
            1.0,
            if selected {
                ORANGE
            } else {
                Color32::from_rgb(52, 53, 59)
            },
        ))
        .corner_radius(8),
    )
}

fn chip(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(11.0).color(if selected {
            ORANGE_SOFT
        } else {
            MUTED
        }))
        .fill(PANEL_2)
        .stroke(Stroke::new(
            1.0,
            if selected {
                ORANGE
            } else {
                Color32::from_rgb(48, 49, 55)
            },
        ))
        .corner_radius(7),
    )
}

fn format_number(value: u64) -> String {
    let text = value.to_string();
    let mut out = String::with_capacity(text.len() + text.len() / 3);
    for (index, ch) in text.chars().enumerate() {
        if index > 0 && (text.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("EmberClick")
            .with_inner_size([760.0, 590.0])
            .with_min_inner_size([700.0, 540.0])
            .with_resizable(true),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "EmberClick",
        options,
        Box::new(|cc| Ok(Box::new(EmberClickApp::new(cc)))),
    )
}
