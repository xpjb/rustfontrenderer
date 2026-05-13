use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytemuck::cast_slice;
use glam::{Mat4, Vec2};
use pollster::block_on;
use text_msdf::{Align, Material, TextArgs, TextEngine, TextRenderer, TextVertex};
use winit::{
    event::{ElementState, Event, KeyEvent, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::WindowBuilder,
};

macro_rules! timed_scope_ms {
    ($label:expr, $sink:expr, $body:block) => {{
        let timed_scope_start = Instant::now();
        let timed_scope_result = { $body };
        $sink.push(($label, timed_scope_start.elapsed().as_secs_f32() * 1000.0));
        timed_scope_result
    }};
}

const WINDOW_W: u32 = 1600;
const WINDOW_H: u32 = 900;
const INITIAL_FONT_SIZE: f32 = 60.0;
const FONT_SIZE_STEP: f32 = 5.0;
const MIN_FONT_SIZE: f32 = 5.0;
const MAX_FONT_SIZE: f32 = 240.0;
const LINE_SPACING: f32 = 1.15;
const INITIAL_FLYER_COUNT: usize = 1800;
const FLYER_STEP: usize = 25;
const MIN_FLYERS: usize = 0;
const MAX_FLYERS: usize = 10000;
const STATS_REFRESH: Duration = Duration::from_millis(200);
const STATS_HISTORY: usize = 720;
const PERF_PRINT_INTERVAL: Duration = Duration::from_secs(1);

const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 0.035,
    g: 0.040,
    b: 0.055,
    a: 1.0,
};
const PHRASES: &[&str] = &[
    "tiny glyph storm",
    "frame time p99",
    "analytic bezier fill",
    "flying text torture",
    "wgpu present immediate",
    "cache the curves",
    "bands and coverage",
    "draw more letters",
    "fps overlay live",
    "subpixel-ish chaos",
    "slug all the things",
    "heap of moving words",
    "abcdefghijklmnopqrstuvwxyz",
    "0123456789 ms p50 p90 p99",
    "colour noise baseline",
    "render pass stays single",
];
const OVERLAY_GLYPH_WARMUP: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 ./:-_()[]x,";

fn main() {
    block_on(run());
}

async fn run() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let font_path = manifest_dir.join("..").join("assets").join("Hack-Regular.ttf");
    let mut engine = TextEngine::load(font_path.to_str().unwrap()).expect("load font");
    let metrics = engine.metrics();
    let line_height_em = metrics.line_height() * LINE_SPACING;

    let phrase_bank = PhraseBank::new(&mut engine);

    let warmup_args = TextArgs {
        size_px: INITIAL_FONT_SIZE,
        color: [1.0, 1.0, 1.0, 1.0],
        max_width_px: None,
        line_spacing: LINE_SPACING,
        align: Align::Left,
        material: Material::Fill,
    };
    engine.text(0.0, 0.0, OVERLAY_GLYPH_WARMUP, &warmup_args);
    let _ = engine.flush();

    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("torture_msdf (Esc quit, Up/Down density, +/- size)")
            .with_inner_size(winit::dpi::PhysicalSize::new(WINDOW_W, WINDOW_H))
            .build(&event_loop)
            .unwrap(),
    );
    let size = window.inner_size();

    let instance = wgpu::Instance::default();
    let surface = instance.create_surface(window.clone()).unwrap();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })
        .await
        .expect("adapter");
    println!("Adapter: {:?}", adapter.get_info());

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default(), None)
        .await
        .expect("device");

    let caps = surface.get_capabilities(&adapter);
    let format = choose_surface_format(&caps);
    let present_mode = choose_present_mode(&caps);
    let alpha_mode = caps
        .alpha_modes
        .first()
        .copied()
        .unwrap_or(wgpu::CompositeAlphaMode::Opaque);

    println!("Surface format: {:?}", format);
    println!("Present mode: {:?}", present_mode);

    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode,
        alpha_mode,
        view_formats: vec![],
        desired_maximum_frame_latency: 1,
    };
    surface.configure(&device, &config);

    let renderer = TextRenderer::new(&device, &config, engine.distance_range_px(), engine.em_to_px());
    let mut atlas = engine.new_atlas(&device, &queue, &renderer.atlas_layout);
    let mut vbuf = DynamicVertexBuffer::new(&device, 4096);

    let mut flyer_count = INITIAL_FLYER_COUNT;
    let mut font_size = INITIAL_FONT_SIZE;
    let mut flyers = build_flyers(flyer_count, &phrase_bank);
    let mut stats = FrameStats::new(STATS_HISTORY);
    let mut overlay = StatsOverlay::new();
    let mut perf = PerfMonitor::new();
    let start = Instant::now();
    let mut last_frame = Instant::now();

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(ControlFlow::Poll);

            match event {
                Event::WindowEvent { event, .. } => match event {
                    WindowEvent::CloseRequested => elwt.exit(),
                    WindowEvent::Resized(size) => {
                        if size.width > 0 && size.height > 0 {
                            config.width = size.width;
                            config.height = size.height;
                            surface.configure(&device, &config);
                        }
                    }
                    WindowEvent::KeyboardInput {
                        event:
                            KeyEvent {
                                physical_key: PhysicalKey::Code(code),
                                state: ElementState::Pressed,
                                ..
                            },
                        ..
                    } => match code {
                        KeyCode::Escape => elwt.exit(),
                        KeyCode::ArrowUp => {
                            flyer_count = (flyer_count + FLYER_STEP).min(MAX_FLYERS);
                            flyers = build_flyers(flyer_count, &phrase_bank);
                            overlay.invalidate();
                        }
                        KeyCode::ArrowDown => {
                            flyer_count = flyer_count.saturating_sub(FLYER_STEP).max(MIN_FLYERS);
                            flyers = build_flyers(flyer_count, &phrase_bank);
                            overlay.invalidate();
                        }
                        KeyCode::ArrowRight | KeyCode::Equal | KeyCode::NumpadAdd => {
                            font_size = (font_size + FONT_SIZE_STEP).min(MAX_FONT_SIZE);
                            overlay.invalidate();
                            println!("Torture font size: {:.0}px", font_size);
                        }
                        KeyCode::ArrowLeft | KeyCode::Minus | KeyCode::NumpadSubtract => {
                            font_size = (font_size - FONT_SIZE_STEP).max(MIN_FONT_SIZE);
                            overlay.invalidate();
                            println!("Torture font size: {:.0}px", font_size);
                        }
                        _ => {}
                    },
                    WindowEvent::RedrawRequested => {
                        let frame_start = Instant::now();
                        let dt_ms = frame_start.duration_since(last_frame).as_secs_f32() * 1000.0;
                        last_frame = frame_start;
                        if dt_ms.is_finite() && dt_ms > 0.0 {
                            stats.push(dt_ms);
                        }

                        let cur = window.inner_size();
                        if cur.width == 0 || cur.height == 0 {
                            return;
                        }

                        let frame = match surface.get_current_texture() {
                            Ok(frame) => frame,
                            Err(err) => {
                                match err {
                                    wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated => {
                                        surface.configure(&device, &config);
                                    }
                                    wgpu::SurfaceError::OutOfMemory => elwt.exit(),
                                    wgpu::SurfaceError::Timeout => {}
                                }
                                return;
                            }
                        };

                        let baseline_origin_px = 12.0 + metrics.ascent * font_size;
                        let world = SceneWorld {
                            width_px: cur.width as f32,
                            height_px: cur.height as f32,
                            baseline_origin_px,
                            line_height_px: line_height_em * font_size,
                            font_size_px: font_size,
                        };
                        let elapsed = start.elapsed().as_secs_f32();

                        let mut frame_scopes = Vec::with_capacity(6);

                        timed_scope_ms!("flyers", frame_scopes, {
                            for flyer in &flyers {
                                let entry = phrase_bank.entry(flyer.phrase_index);
                                let baseline =
                                    flyer.baseline_at(elapsed, &world, entry.width_em * font_size);
                                let color = animated_color(flyer.hue + elapsed * flyer.hue_rate, 0.95);
                                let mut args = entry.args.clone();
                                args.size_px = font_size;
                                args.color = color;
                                engine.text(baseline.x, baseline.y, entry.phrase, &args);
                            }
                        });

                        let summary = stats.summary();
                        if overlay.should_refresh(frame_start) {
                            timed_scope_ms!("overlay_rebuild", frame_scopes, {
                                overlay.refresh_strings(
                                    &summary,
                                    &phrase_bank,
                                    &world,
                                    flyer_count,
                                    present_mode,
                                );
                            });
                        }
                        overlay.emit(&mut engine, &world, line_height_em);
                        timed_scope_ms!("atlas_sync", frame_scopes, {
                            engine.sync_atlas(&mut atlas, &device, &queue, &renderer.atlas_layout);
                        });

                        let verts = timed_scope_ms!("flush", frame_scopes, {
                            engine.flush()
                        });
                        let vertex_count = verts.len();
                        let build_ms = frame_scopes.iter().map(|(_, ms)| *ms).sum::<f32>();

                        timed_scope_ms!("vertex_upload", frame_scopes, {
                            vbuf.write(&device, &queue, verts);
                        });

                        let view = frame.texture.create_view(&Default::default());
                        let mut encoder = device.create_command_encoder(&Default::default());

                        let matrix = Mat4::orthographic_rh(
                            0.0,
                            cur.width as f32,
                            cur.height as f32,
                            0.0,
                            -1.0,
                            1.0,
                        );

                        timed_scope_ms!("encode_render", frame_scopes, {
                            renderer.render(
                                &queue,
                                &mut encoder,
                                &view,
                                &atlas,
                                vbuf.buffer(),
                                vertex_count as u32,
                                matrix,
                                (cur.width, cur.height),
                                Some(BACKGROUND),
                            );
                        });

                        timed_scope_ms!("submit_present", frame_scopes, {
                            queue.submit(std::iter::once(encoder.finish()));
                            frame.present();
                        });

                        let upload_ms = frame_scopes
                            .iter()
                            .find(|(label, _)| *label == "vertex_upload")
                            .map(|(_, ms)| *ms)
                            .unwrap_or(0.0);
                        let submit_ms = frame_scopes
                            .iter()
                            .filter(|(label, _)| *label == "encode_render" || *label == "submit_present")
                            .map(|(_, ms)| *ms)
                            .sum::<f32>();

                        perf.record(FramePerfSample {
                            total_ms: frame_start.elapsed().as_secs_f32() * 1000.0,
                            build_ms,
                            upload_ms,
                            submit_ms,
                            vertex_count,
                            scopes: frame_scopes,
                        });
                        perf.maybe_print(flyer_count, present_mode);
                    }
                    _ => {}
                },
                Event::AboutToWait => window.request_redraw(),
                _ => {}
            }
        })
        .unwrap();
}

struct SceneWorld {
    width_px: f32,
    height_px: f32,
    baseline_origin_px: f32,
    line_height_px: f32,
    font_size_px: f32,
}

struct PhraseEntry {
    phrase: &'static str,
    width_em: f32,
    args: TextArgs,
}

struct PhraseBank {
    entries: Vec<PhraseEntry>,
}

impl PhraseBank {
    fn new(engine: &mut TextEngine) -> Self {
        let measure_args = TextArgs {
            size_px: 1.0,
            color: [1.0, 1.0, 1.0, 1.0],
            max_width_px: None,
            line_spacing: LINE_SPACING,
            align: Align::Left,
            material: Material::Fill,
        };
        let template_args = TextArgs {
            size_px: INITIAL_FONT_SIZE,
            color: [1.0, 1.0, 1.0, 1.0],
            max_width_px: None,
            line_spacing: LINE_SPACING,
            align: Align::Left,
            material: Material::Fill,
        };
        let mut entries = Vec::with_capacity(PHRASES.len());
        for phrase in PHRASES {
            let m = engine.measure(phrase, &measure_args);
            entries.push(PhraseEntry {
                phrase,
                width_em: m.width_px,
                args: template_args.clone(),
            });
        }
        Self { entries }
    }

    fn entry(&self, index: usize) -> &PhraseEntry {
        &self.entries[index]
    }
}

#[derive(Clone, Copy)]
struct Flyer {
    phrase_index: usize,
    start_x_px: f32,
    start_y_px: f32,
    speed_x_px: f32,
    speed_y_px: f32,
    wobble_amp_px: f32,
    wobble_freq_hz: f32,
    wobble_phase: f32,
    hue: f32,
    hue_rate: f32,
}

impl Flyer {
    fn baseline_at(&self, elapsed: f32, world: &SceneWorld, text_width_px: f32) -> Vec2 {
        let span_x = world.width_px + text_width_px + 80.0;
        let span_y = world.height_px + 60.0;
        let x = wrap_range(
            self.start_x_px + elapsed * self.speed_x_px,
            -text_width_px - 40.0,
            -text_width_px - 40.0 + span_x,
        );
        let drift_y = self.start_y_px + elapsed * self.speed_y_px;
        let wobble = (elapsed * self.wobble_freq_hz + self.wobble_phase).sin() * self.wobble_amp_px;
        let y = wrap_range(drift_y + wobble, 18.0, 18.0 + span_y);
        Vec2::new(x, y)
    }
}

fn build_flyers(count: usize, bank: &PhraseBank) -> Vec<Flyer> {
    let mut flyers = Vec::with_capacity(count);
    for i in 0..count {
        let i = i as u32;
        flyers.push(Flyer {
            phrase_index: hash_index(i.wrapping_mul(17).wrapping_add(3), bank.entries.len()),
            start_x_px: hash01(i.wrapping_mul(101).wrapping_add(11)) * WINDOW_W as f32,
            start_y_px: hash01(i.wrapping_mul(131).wrapping_add(19)) * WINDOW_H as f32,
            speed_x_px: lerp(70.0, 300.0, hash01(i.wrapping_mul(149).wrapping_add(23)))
                * if (i & 1) == 0 { 1.0 } else { -1.0 },
            speed_y_px: lerp(-22.0, 22.0, hash01(i.wrapping_mul(173).wrapping_add(29))),
            wobble_amp_px: lerp(0.0, 22.0, hash01(i.wrapping_mul(197).wrapping_add(31))),
            wobble_freq_hz: lerp(0.7, 3.1, hash01(i.wrapping_mul(211).wrapping_add(37))),
            wobble_phase: hash01(i.wrapping_mul(239).wrapping_add(41)) * std::f32::consts::TAU,
            hue: hash01(i.wrapping_mul(251).wrapping_add(43)),
            hue_rate: lerp(0.015, 0.085, hash01(i.wrapping_mul(269).wrapping_add(47))),
        });
    }
    flyers
}

struct DynamicVertexBuffer {
    buffer: wgpu::Buffer,
    capacity: usize,
}

impl DynamicVertexBuffer {
    fn new(device: &wgpu::Device, initial_capacity: usize) -> Self {
        let capacity = initial_capacity.max(1);
        Self {
            buffer: create_vertex_buffer(device, capacity),
            capacity,
        }
    }

    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, vertices: &[TextVertex]) {
        if vertices.len() > self.capacity {
            self.capacity = vertices.len().next_power_of_two();
            self.buffer = create_vertex_buffer(device, self.capacity);
        }
        if !vertices.is_empty() {
            queue.write_buffer(&self.buffer, 0, cast_slice(vertices));
        }
    }

    fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

fn create_vertex_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    let size = (capacity.max(1) * std::mem::size_of::<TextVertex>()) as u64;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("torture dynamic vertex buffer"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

struct FrameStats {
    samples_ms: VecDeque<f32>,
    max_samples: usize,
}

struct PerfMonitor {
    samples: VecDeque<FramePerfSample>,
    last_print: Instant,
}

#[derive(Clone, Default)]
struct FramePerfSample {
    total_ms: f32,
    build_ms: f32,
    upload_ms: f32,
    submit_ms: f32,
    vertex_count: usize,
    scopes: Vec<(&'static str, f32)>,
}

impl PerfMonitor {
    fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(240),
            last_print: Instant::now(),
        }
    }

    fn record(&mut self, sample: FramePerfSample) {
        if self.samples.len() == 240 {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    fn maybe_print(&mut self, flyer_count: usize, present_mode: wgpu::PresentMode) {
        if self.last_print.elapsed() < PERF_PRINT_INTERVAL || self.samples.is_empty() {
            return;
        }
        self.last_print = Instant::now();

        let n = self.samples.len() as f32;
        let avg_total = self.samples.iter().map(|s| s.total_ms).sum::<f32>() / n;
        let avg_build = self.samples.iter().map(|s| s.build_ms).sum::<f32>() / n;
        let avg_upload = self.samples.iter().map(|s| s.upload_ms).sum::<f32>() / n;
        let avg_submit = self.samples.iter().map(|s| s.submit_ms).sum::<f32>() / n;
        let avg_vertices = self.samples.iter().map(|s| s.vertex_count as f32).sum::<f32>() / n;
        let mb_per_frame =
            avg_vertices * std::mem::size_of::<TextVertex>() as f32 / (1024.0 * 1024.0);
        let mut scope_sums: Vec<(&'static str, f32)> = Vec::new();
        for sample in &self.samples {
            for (label, ms) in &sample.scopes {
                if let Some((_, total)) = scope_sums.iter_mut().find(|(existing, _)| existing == label) {
                    *total += *ms;
                } else {
                    scope_sums.push((*label, *ms));
                }
            }
        }
        scope_sums.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let scope_summary = scope_sums
            .iter()
            .map(|(label, total_ms)| format!("{}={:.2}ms", label, total_ms / n))
            .collect::<Vec<_>>()
            .join(" ");

        println!(
            "perf flyers={} mode={:?} avg_total={:.2}ms build={:.2}ms upload={:.2}ms submit={:.2}ms verts/frame={:.0} upload/frame={:.2} MiB {}",
            flyer_count,
            present_mode,
            avg_total,
            avg_build,
            avg_upload,
            avg_submit,
            avg_vertices,
            mb_per_frame,
            scope_summary,
        );
    }
}

impl FrameStats {
    fn new(max_samples: usize) -> Self {
        Self {
            samples_ms: VecDeque::with_capacity(max_samples),
            max_samples,
        }
    }

    fn push(&mut self, frame_ms: f32) {
        if self.samples_ms.len() == self.max_samples {
            self.samples_ms.pop_front();
        }
        self.samples_ms.push_back(frame_ms);
    }

    fn summary(&self) -> FrameSummary {
        if self.samples_ms.is_empty() {
            return FrameSummary::default();
        }

        let mut sorted: Vec<f32> = self.samples_ms.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let avg_ms = self.samples_ms.iter().sum::<f32>() / self.samples_ms.len() as f32;
        let latest_ms = *self.samples_ms.back().unwrap();
        let fps = if avg_ms > 0.0 { 1000.0 / avg_ms } else { 0.0 };

        FrameSummary {
            fps,
            latest_ms,
            avg_ms,
            p50_ms: percentile(&sorted, 0.50),
            p90_ms: percentile(&sorted, 0.90),
            p99_ms: percentile(&sorted, 0.99),
            max_ms: *sorted.last().unwrap(),
            samples: self.samples_ms.len(),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct FrameSummary {
    fps: f32,
    latest_ms: f32,
    avg_ms: f32,
    p50_ms: f32,
    p90_ms: f32,
    p99_ms: f32,
    max_ms: f32,
    samples: usize,
}

fn percentile(sorted: &[f32], fraction: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f32 * fraction).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

struct StatsOverlay {
    last_refresh: Instant,
    hud_lines: Vec<String>,
    footer: String,
}

impl StatsOverlay {
    fn new() -> Self {
        Self {
            last_refresh: Instant::now() - STATS_REFRESH,
            hud_lines: Vec::new(),
            footer: String::new(),
        }
    }

    fn should_refresh(&self, now: Instant) -> bool {
        now.duration_since(self.last_refresh) >= STATS_REFRESH
    }

    fn invalidate(&mut self) {
        self.last_refresh = Instant::now() - STATS_REFRESH;
    }

    #[allow(clippy::too_many_arguments)]
    fn refresh_strings(
        &mut self,
        stats: &FrameSummary,
        bank: &PhraseBank,
        world: &SceneWorld,
        flyer_count: usize,
        present_mode: wgpu::PresentMode,
    ) {
        self.last_refresh = Instant::now();

        self.hud_lines.clear();
        self.hud_lines.push(format!(
            "fps {:7.1}   frame {:6.2} ms   avg {:6.2} ms",
            stats.fps, stats.latest_ms, stats.avg_ms
        ));
        self.hud_lines.push(format!(
            "p50 {:6.2}   p90 {:6.2}   p99 {:6.2}   max {:6.2}",
            stats.p50_ms, stats.p90_ms, stats.p99_ms, stats.max_ms
        ));
        self.hud_lines.push(format!(
            "samples {:4}   flyers {:5}   phrases {:2}",
            stats.samples,
            flyer_count,
            bank.entries.len(),
        ));
        self.hud_lines
            .push(format!("present {:?}   poll uncapped", present_mode));
        self.hud_lines.push(format!(
            "font {:.0}px   Up/Down density   Left/Right size   Esc quits",
            world.font_size_px
        ));

        self.footer = format!(
            "window {:.0}x{:.0}   baseline {:.1}px   line {:.1}px",
            world.width_px, world.height_px, world.baseline_origin_px, world.line_height_px
        );
    }

    fn emit(&self, engine: &mut TextEngine, world: &SceneWorld, line_height_em: f32) {
        if self.hud_lines.is_empty() {
            return;
        }

        let padding_left_px = 12.0;
        let line_step_px = line_height_em * world.font_size_px;

        let hud_args = TextArgs {
            size_px: world.font_size_px,
            color: [0.97, 0.97, 0.98, 1.0],
            max_width_px: None,
            line_spacing: LINE_SPACING,
            align: Align::Left,
            material: Material::Fill,
        };

        for (i, line) in self.hud_lines.iter().enumerate() {
            let baseline_y = world.baseline_origin_px + i as f32 * line_step_px;
            engine.text(padding_left_px, baseline_y, line.as_str(), &hud_args);
        }

        let footer_baseline_px =
            world.baseline_origin_px + (self.hud_lines.len() as f32 + 0.35) * line_step_px;
        let footer_args = TextArgs {
            size_px: world.font_size_px,
            color: [0.82, 0.86, 0.91, 1.0],
            max_width_px: None,
            line_spacing: LINE_SPACING,
            align: Align::Left,
            material: Material::Fill,
        };
        engine.text(padding_left_px, footer_baseline_px, self.footer.as_str(), &footer_args);
    }
}

fn choose_surface_format(caps: &wgpu::SurfaceCapabilities) -> wgpu::TextureFormat {
    caps.formats
        .iter()
        .copied()
        .find(|format| format.is_srgb())
        .unwrap_or(caps.formats[0])
}

fn choose_present_mode(caps: &wgpu::SurfaceCapabilities) -> wgpu::PresentMode {
    let preferred = [
        wgpu::PresentMode::Immediate,
        wgpu::PresentMode::Mailbox,
        wgpu::PresentMode::AutoNoVsync,
        wgpu::PresentMode::FifoRelaxed,
        wgpu::PresentMode::Fifo,
    ];
    preferred
        .into_iter()
        .find(|mode| caps.present_modes.contains(mode))
        .unwrap_or(caps.present_modes[0])
}

fn animated_color(hue: f32, alpha: f32) -> [f32; 4] {
    let hue = hue.fract().rem_euclid(1.0);
    let h = hue * 6.0;
    let c = 0.85;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + 0.15, g + 0.15, b + 0.15, alpha]
}

fn wrap_range(value: f32, min: f32, max: f32) -> f32 {
    let span = (max - min).max(1.0);
    min + (value - min).rem_euclid(span)
}

fn lerp(min: f32, max: f32, t: f32) -> f32 {
    min + (max - min) * t
}

fn hash_index(seed: u32, len: usize) -> usize {
    ((hash01(seed) * len as f32) as usize).min(len.saturating_sub(1))
}

fn hash01(mut x: u32) -> f32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x as f32 / u32::MAX as f32
}
