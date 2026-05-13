//! Demo: loads Hack, wraps a paragraph to a fixed width, and renders it
//! as solid-filled text with the Slug coverage shader.
//!
//! Keys: Esc quit.

use std::path::PathBuf;
use std::sync::Arc;

use glam::Mat4;
use pollster::block_on;
use text::{Align, TextArgs, TextEngine, TextRenderer};
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
line breaker -- no kerning across the break, no Unicode rules, just words.";

fn main() {
    block_on(run());
}

async fn run() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let font_path = manifest_dir.join("..").join("assets").join("Hack-Regular.ttf");
    let mut engine = TextEngine::load(font_path.to_str().unwrap()).expect("load font");

    let column_w_px = WINDOW_W as f32 - MARGIN * 2.0;
    let metrics = engine.metrics();
    let measured = engine.measure(
        SAMPLE,
        &TextArgs {
            size_px: FONT_SIZE,
            color: [0.0; 4],
            max_width_px: Some(column_w_px),
            line_spacing: LINE_SPACING,
            align: Align::Left,
        },
    );

    println!(
        "Wrapped block: {:.0}px wide × {:.0}px tall, {} lines (column = {:.0}px)",
        measured.width_px, measured.height_px, measured.line_count, column_w_px
    );
    println!(
        "Cache: curve tex {:?}; band tex {:?}",
        engine.curve_atlas_size(),
        engine.band_atlas_size()
    );

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
    let mut atlas = engine.new_atlas(&device, &queue, &renderer.atlas_layout);

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
                    if cur.width == 0 || cur.height == 0 {
                        return;
                    }

                    let baseline_y = MARGIN + metrics.ascent * FONT_SIZE;

                    let args = TextArgs {
                        size_px: FONT_SIZE,
                        color: [0.10, 0.10, 0.12, 1.0],
                        max_width_px: Some(column_w_px),
                        line_spacing: LINE_SPACING,
                        align: Align::Left,
                    };
                    engine.text(MARGIN, baseline_y, SAMPLE, &args);

                    engine.sync_atlas(&mut atlas, &device, &queue, &renderer.atlas_layout);

                    let verts = engine.flush();
                    let vbuf = TextRenderer::build_vertices(&device, verts);

                    let frame = match surface.get_current_texture() {
                        Ok(f) => f,
                        Err(_) => return,
                    };
                    let view = frame.texture.create_view(&Default::default());
                    let mut encoder = device.create_command_encoder(&Default::default());

                    let matrix = Mat4::orthographic_rh(
                        0.0, cur.width as f32, cur.height as f32, 0.0, -1.0, 1.0,
                    );

                    let bg = wgpu::Color { r: 0.95, g: 0.95, b: 0.97, a: 1.0 };
                    renderer.render(
                        &queue,
                        &mut encoder,
                        &view,
                        &atlas,
                        &vbuf,
                        verts.len() as u32,
                        matrix,
                        (cur.width, cur.height),
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
