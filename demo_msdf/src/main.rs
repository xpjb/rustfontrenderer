//! MSDF material swatches: fill + parameter sweeps. Layout scales with window size.
//!
//! Keys: **Esc** quit.

use std::path::PathBuf;
use std::sync::Arc;

use glam::Mat4;
use pollster::block_on;
use text_msdf::{Align, Material, TextArgs, TextEngine, TextRenderer};
use winit::{
    event::{ElementState, Event, KeyEvent, WindowEvent},
    event_loop::EventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::WindowBuilder,
};

const LINE_SPACING: f32 = 1.08;
const INITIAL_W: u32 = 1440;
const INITIAL_H: u32 = 1080;

struct FrameLayout {
    w: f32,
    margin_x: f32,
    label_px: f32,
    swatch_px: f32,
    sample_gap: f32,
    footer_px: f32,
    body_start_y: f32,
    footer_y: f32,
    /// Delta from section header `y` to swatch row `y`.
    header_body_gap: f32,
    row_step: f32,
}

impl FrameLayout {
    fn new(width: u32, height: u32) -> Self {
        let w = width.max(1) as f32;
        let h = height.max(1) as f32;
        let min_dim = w.min(h);
        let scale = (min_dim / 760.0).clamp(0.85, 2.8);

        let margin_x = w * 0.028;
        let margin_y = h * 0.018;

        let label_px = (26.0 * scale).clamp(18.0, 44.0);
        let swatch_px = (min_dim * 0.088).clamp(40.0, 120.0);
        let sample_gap = (min_dim * 0.032).clamp(18.0, 60.0);
        // Extra px between each material row (not text line spacing — unrelated to em units).
        // `0.012 * height` ≈ 13px at 1080p (~1.2% of window height), not a fraction of em.
        let between_rows_px = (h * 0.012).max(10.0);
        let footer_px = (18.0 * scale).clamp(12.0, 28.0);

        let body_start_y = margin_y + 6.0;

        // Section label → swatches: room for label descenders + outline/glow bleed above swatch row.
        let header_body_gap = label_px * 0.55 + swatch_px * 0.48 + 8.0;
        let row_step = header_body_gap + swatch_px * LINE_SPACING + between_rows_px;
        let footer_y = h - margin_y - footer_px * 0.35;

        Self {
            w,
            margin_x,
            label_px,
            swatch_px,
            sample_gap,
            footer_px,
            body_start_y,
            footer_y,
            header_body_gap,
            row_step,
        }
    }
}

fn main() {
    block_on(run());
}

fn measure_args(size_px: f32) -> TextArgs {
    TextArgs {
        size_px,
        color: [1.0, 1.0, 1.0, 1.0],
        max_width_px: None,
        line_spacing: LINE_SPACING,
        align: Align::Left,
        material: Material::Fill,
    }
}

fn row_title(engine: &mut TextEngine, lay: &FrameLayout, y: f32, title: &str) {
    let mut a = measure_args(lay.label_px);
    a.color = [0.72, 0.76, 0.84, 1.0];
    engine.text(lay.margin_x, y, title, &a);
}

fn place_swatches(
    engine: &mut TextEngine,
    lay: &FrameLayout,
    y: f32,
    samples: &[(&str, Material)],
) {
    let mut a = measure_args(lay.swatch_px);
    a.color = [0.93, 0.94, 0.97, 1.0];
    let mut x = lay.margin_x;
    let max_x = lay.w - lay.margin_x;
    for (label, mat) in samples {
        a.material = *mat;
        let adv = engine.measure(label, &a).width_px + lay.sample_gap;
        if x > lay.margin_x && x + adv > max_x {
            break;
        }
        engine.text(x, y, label, &a);
        x += adv;
    }
}

async fn run() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let font_path = manifest_dir.join("..").join("assets").join("Hack-Regular.ttf");
    let mut engine = TextEngine::load(font_path.to_str().unwrap()).expect("load font");

    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("demo_msdf — materials (Esc) · resize to scale")
            .with_inner_size(winit::dpi::PhysicalSize::new(INITIAL_W, INITIAL_H))
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

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default(), None)
        .await
        .expect("device");

    let caps = surface.get_capabilities(&adapter);
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(caps.formats[0]);

    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: caps
            .alpha_modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Opaque),
        view_formats: vec![],
        desired_maximum_frame_latency: 1,
    };
    surface.configure(&device, &config);

    let renderer = TextRenderer::new(&device, &config, engine.distance_range_px());
    let atlas = engine.new_atlas(&device, &queue, &renderer.atlas_layout);

    let bg = wgpu::Color {
        r: 0.06,
        g: 0.07,
        b: 0.10,
        a: 1.0,
    };

    let outline_c = [0.15, 0.72, 1.0, 1.0];
    let glow_c = [0.1, 0.95, 0.55, 1.0];
    let shadow_c = [0.0, 0.0, 0.0, 0.72];

    event_loop
        .run(move |event, elwt| {
            use winit::event_loop::ControlFlow;
            elwt.set_control_flow(ControlFlow::Poll);
            match event {
                Event::WindowEvent { event, .. } => match event {
                    WindowEvent::CloseRequested => elwt.exit(),
                    WindowEvent::Resized(sz) => {
                        if sz.width > 0 && sz.height > 0 {
                            config.width = sz.width;
                            config.height = sz.height;
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
                    } => {
                        if code == KeyCode::Escape {
                            elwt.exit();
                        }
                    }
                    WindowEvent::RedrawRequested => {
                        let sz = window.inner_size();
                        if sz.width == 0 || sz.height == 0 {
                            return;
                        }

                        let lay = FrameLayout::new(sz.width, sz.height);

                        let mut y = lay.body_start_y;

                        row_title(&mut engine, &lay, y, "[1] FILL — baseline");
                        y += lay.header_body_gap;
                        place_swatches(
                            &mut engine,
                            &lay,
                            y,
                            &[("fill ABC fi-xyz", Material::Fill)],
                        );
                        y += lay.row_step;

                        row_title(
                            &mut engine,
                            &lay,
                            y,
                            "[2] OUTLINE — width_px (cyan ring)",
                        );
                        y += lay.header_body_gap;
                        place_swatches(
                            &mut engine,
                            &lay,
                            y,
                            &[
                                (
                                    "w=1",
                                    Material::Outline {
                                        width_px: 1.0,
                                        color: outline_c,
                                    },
                                ),
                                (
                                    "w=2",
                                    Material::Outline {
                                        width_px: 2.0,
                                        color: outline_c,
                                    },
                                ),
                                (
                                    "w=4",
                                    Material::Outline {
                                        width_px: 4.0,
                                        color: outline_c,
                                    },
                                ),
                                (
                                    "w=8",
                                    Material::Outline {
                                        width_px: 8.0,
                                        color: outline_c,
                                    },
                                ),
                            ],
                        );
                        y += lay.row_step;

                        row_title(
                            &mut engine,
                            &lay,
                            y,
                            "[3] GLOW — radius_px, strength",
                        );
                        y += lay.header_body_gap;
                        place_swatches(
                            &mut engine,
                            &lay,
                            y,
                            &[
                                (
                                    "r=2 s=0.5",
                                    Material::Glow {
                                        radius_px: 2.0,
                                        color: glow_c,
                                        strength: 0.5,
                                    },
                                ),
                                (
                                    "r=4 s=1",
                                    Material::Glow {
                                        radius_px: 4.0,
                                        color: glow_c,
                                        strength: 1.0,
                                    },
                                ),
                                (
                                    "r=8 s=1.5",
                                    Material::Glow {
                                        radius_px: 8.0,
                                        color: glow_c,
                                        strength: 1.5,
                                    },
                                ),
                                (
                                    "r=12 s=2",
                                    Material::Glow {
                                        radius_px: 12.0,
                                        color: glow_c,
                                        strength: 2.0,
                                    },
                                ),
                            ],
                        );
                        y += lay.row_step;

                        row_title(
                            &mut engine,
                            &lay,
                            y,
                            "[4] SHADOW — offset_px (screen), blur_px",
                        );
                        y += lay.header_body_gap;
                        place_swatches(
                            &mut engine,
                            &lay,
                            y,
                            &[
                                (
                                    "o=6,7 b=2",
                                    Material::Shadow {
                                        offset_px: [6.0, 7.0],
                                        blur_px: 2.0,
                                        color: shadow_c,
                                    },
                                ),
                                (
                                    "o=12,14 b=4",
                                    Material::Shadow {
                                        offset_px: [12.0, 14.0],
                                        blur_px: 4.0,
                                        color: shadow_c,
                                    },
                                ),
                                (
                                    "o=22,26 b=8",
                                    Material::Shadow {
                                        offset_px: [22.0, 26.0],
                                        blur_px: 8.0,
                                        color: shadow_c,
                                    },
                                ),
                            ],
                        );

                        let mut foot = measure_args(lay.footer_px);
                        foot.color = [0.48, 0.52, 0.58, 1.0];
                        foot.max_width_px = Some(lay.w - lay.margin_x * 2.0);
                        engine.text(
                            lay.margin_x,
                            lay.footer_y,
                            "? = charset. fi shown without ligature (liga/clig off).",
                            &foot,
                        );

                        let verts = engine.flush();
                        let n = verts.len() as u32;
                        let vbuf = TextRenderer::build_vertices(&device, verts);

                        let frame = match surface.get_current_texture() {
                            Ok(f) => f,
                            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                                surface.configure(&device, &config);
                                return;
                            }
                            Err(wgpu::SurfaceError::OutOfMemory) => {
                                elwt.exit();
                                return;
                            }
                            Err(wgpu::SurfaceError::Timeout) => return,
                        };

                        let view = frame.texture.create_view(&Default::default());
                        let mut enc = device.create_command_encoder(&Default::default());
                        let matrix = Mat4::orthographic_rh(
                            0.0,
                            sz.width as f32,
                            sz.height as f32,
                            0.0,
                            -1.0,
                            1.0,
                        );
                        renderer.render(
                            &queue,
                            &mut enc,
                            &view,
                            &atlas,
                            &vbuf,
                            n,
                            matrix,
                            (sz.width, sz.height),
                            Some(bg),
                        );
                        queue.submit(std::iter::once(enc.finish()));
                        frame.present();
                    }
                    _ => {}
                },
                Event::AboutToWait => window.request_redraw(),
                _ => {}
            }
        })
        .unwrap();
}
