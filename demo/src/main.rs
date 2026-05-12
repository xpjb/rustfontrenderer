//! Demo: loads NotoSansSC, wraps a paragraph to a fixed width, and renders it
//! as solid-filled text with the Slug coverage shader.
//!
//! Keys: Esc quit.

use std::path::PathBuf;
use std::sync::Arc;

use glam::{Mat4, Vec3};
use pollster::block_on;
use text::{break_lines, shape_text, Font, GlyphCache, TextAtlas, TextRenderer};
use winit::{
    event::{ElementState, Event, KeyEvent, WindowEvent},
    event_loop::EventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::WindowBuilder,
};

const FONT_SIZE: f32 = 64.0;
const LINE_SPACING: f32 = 1.25;
const WINDOW_W: u32 = 900;
const WINDOW_H: u32 = 700;
const MARGIN: f32 = 40.0;

const SAMPLE: &str = "The quick brown fox jumps over the lazy dog. \
Slug renders glyphs directly from quadratic Bezier curves on the GPU, \
using horizontal and vertical bands to make per-pixel coverage cheap. \
This paragraph is wrapped to a fixed pixel width by a naive whitespace \
line breaker â€” no kerning across the break, no Unicode rules, just words.";

fn main() {
    block_on(run());
}

async fn run() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let font_path = manifest_dir.join("..").join("assets").join("NotoSansSC-Regular.ttf");
    let font = Font::load(font_path.to_str().unwrap()).expect("load font");

    let column_w_px = WINDOW_W as f32 - MARGIN * 2.0;
    let column_w_em = column_w_px / FONT_SIZE;
    let lines = break_lines(&font, SAMPLE, column_w_em);

    let mut cache = GlyphCache::new();
    let metrics = font.metrics();
    let line_height_em = metrics.line_height() * LINE_SPACING;

    let mut runs = Vec::new();
    let mut y_em = 0.0;
    for line in &lines {
        let run = shape_text(&font, &mut cache, &line.text, 0.0, y_em);
        runs.push(run);
        y_em -= line_height_em;
    }

    println!("Wrapped to {} lines (column = {:.1}em / {:.0}px):", lines.len(), column_w_em, column_w_px);
    for (i, line) in lines.iter().enumerate() {
        println!("  [{:>2}] adv={:5.2}em  {:?}", i, line.advance, line.text);
    }
    println!("Cache: {} glyph(s); curve tex {:?}; band tex {:?}",
        runs.iter().map(|r| r.glyphs.len()).sum::<usize>(),
        cache.curve_size(), cache.band_size());

    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("rustfontrenderer demo (Esc = quit)")
            .with_inner_size(winit::dpi::PhysicalSize::new(WINDOW_W, WINDOW_H))
            .build(&event_loop)
            .unwrap(),
    );
    let size = window.inner_size();

    let instance = wgpu::Instance::default();
    let surface = instance.create_surface(window.clone()).unwrap();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
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

    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Opaque,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);

    let renderer = TextRenderer::new(&device, &config);
    let atlas = TextAtlas::new(&device, &queue, &renderer.atlas_layout, &cache);

    let color = [0.10, 0.10, 0.12, 1.0];
    let runs_ref: Vec<_> = runs.iter().map(|r| (r, color)).collect();
    let vertices = text::build_run_vertices(&runs_ref);
    let vbuf = TextRenderer::build_vertices(&device, &vertices);
    let count = vertices.len() as u32;

    window.request_redraw();

    event_loop
        .run(move |event, elwt| match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => elwt.exit(),
                WindowEvent::Resized(s) => {
                    config.width = s.width.max(1);
                    config.height = s.height.max(1);
                    surface.configure(&device, &config);
                }
                WindowEvent::KeyboardInput {
                    event: KeyEvent { physical_key: PhysicalKey::Code(code), state: ElementState::Pressed, .. },
                    ..
                } => match code {
                    KeyCode::Escape => elwt.exit(),
                    _ => {}
                },
                WindowEvent::RedrawRequested => {
                    let cur = window.inner_size();
                    if cur.width == 0 || cur.height == 0 { return; }
                    let frame = match surface.get_current_texture() {
                        Ok(f) => f,
                        Err(_) => return,
                    };
                    let view = frame.texture.create_view(&Default::default());
                    let mut encoder = device.create_command_encoder(&Default::default());

                    let proj = Mat4::orthographic_rh(
                        0.0, cur.width as f32, cur.height as f32, 0.0, -1.0, 1.0,
                    );
                    let baseline_y = MARGIN + metrics.ascent * FONT_SIZE;
                    let model = Mat4::from_translation(Vec3::new(MARGIN, baseline_y, 0.0))
                        * Mat4::from_scale(Vec3::new(FONT_SIZE, -FONT_SIZE, 1.0));
                    let matrix = proj * model;

                    let bg = wgpu::Color { r: 0.95, g: 0.95, b: 0.97, a: 1.0 };
                    renderer.render(
                        &queue, &mut encoder, &view, &atlas,
                        &vbuf, count,
                        matrix, (cur.width, cur.height),
                        Some(bg),
                    );

                    queue.submit(std::iter::once(encoder.finish()));
                    frame.present();
                }
                _ => {}
            },
            Event::AboutToWait => window.request_redraw(),
            _ => {}
        })
        .unwrap();
}
