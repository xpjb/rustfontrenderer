//! Codegen for MSDF phase 2: WGSL material plugins + `Material` enum.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use naga::valid::{Capabilities, ValidationFlags, Validator};
use serde::Deserialize;

/// Stable tag order: built-ins first, then any other `*.wgsl` lexicographically.
const CANONICAL_MATERIALS: &[&str] = &["fill", "outline", "glow", "shadow"];

#[derive(Debug, Deserialize)]
struct Header {
    name: String,
    #[serde(default)]
    params: Vec<ParamSpec>,
}

#[derive(Debug, Deserialize)]
struct ParamSpec {
    ident: String,
    ty: String,
}

struct ParsedMaterial {
    stem: String,
    variant: String,
    header: Header,
    /// WGSL body (`fn material_…` onward).
    body: String,
}

fn param_slot_count(ty: &str) -> usize {
    match ty.trim() {
        "f32" => 1,
        "vec2<f32>" => 2,
        "vec4<f32>" => 4,
        _ => panic!("unsupported material param type {:?}", ty),
    }
}

fn rust_field_type(ty: &str) -> &'static str {
    match ty.trim() {
        "f32" => "f32",
        "vec2<f32>" => "[f32; 2]",
        "vec4<f32>" => "[f32; 4]",
        _ => panic!("unsupported material param type {:?}", ty),
    }
}

fn to_variant_name(stem: &str) -> String {
    let mut ch = stem.chars();
    match ch.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
    }
}

fn split_header_body(src: &str) -> (String, String) {
    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0usize;
    let mut doc = String::new();
    while i < lines.len() {
        let t = lines[i].trim_start();
        if let Some(rest) = t.strip_prefix("//!") {
            doc.push_str(rest.trim_start());
            doc.push('\n');
            i += 1;
            continue;
        }
        if doc.is_empty() && t.is_empty() {
            i += 1;
            continue;
        }
        break;
    }
    if doc.is_empty() {
        panic!("material file must start with `//!` TOML header");
    }
    while i < lines.len() && lines[i].trim().is_empty() {
        i += 1;
    }
    let body = lines[i..].join("\n");
    (doc, body)
}

fn load_material(path: &Path) -> ParsedMaterial {
    let src = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {:?}: {}", path, e));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("material")
        .to_string();
    let (doc, body) = split_header_body(&src);
    let header: Header =
        toml::from_str(doc.trim()).expect("parse material `//!` header as TOML");
    let exp = expected_fn(&stem);
    if !body.contains(&format!("fn {}", exp)) {
        panic!("{:?}: expected `fn {}` in body", path, exp);
    }
    ParsedMaterial {
        variant: to_variant_name(&stem),
        stem,
        header,
        body: body.trim().to_string(),
    }
}

fn expected_fn(stem: &str) -> String {
    format!("material_{}", stem.replace('-', "_"))
}

fn collect_material_wgsl(materials_dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for stem in CANONICAL_MATERIALS {
        let p = materials_dir.join(format!("{}.wgsl", stem));
        assert!(p.is_file(), "missing required material {:?}", p);
        found.push(p);
    }
    let canonical: HashSet<_> = CANONICAL_MATERIALS
        .iter()
        .map(|s| materials_dir.join(format!("{}.wgsl", s)))
        .collect();
    let rd = fs::read_dir(materials_dir).expect("read materials dir");
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("wgsl") {
            continue;
        }
        if canonical.contains(&p) {
            continue;
        }
        found.push(p);
    }
    let n_canon = CANONICAL_MATERIALS.len();
    found[n_canon..].sort_by(|a, b| {
        a.file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .cmp(b.file_stem().unwrap().to_str().unwrap())
    });
    found
}

fn validate_header(m: &ParsedMaterial) {
    let expect = to_variant_name(&m.stem);
    if m.header.name != expect {
        panic!(
            "{}: header name {:?} should match variant {:?} (from filename)",
            m.stem, m.header.name, expect
        );
    }
    let mut used = 0usize;
    for p in &m.header.params {
        let add = param_slot_count(&p.ty);
        used += add;
    }
    assert!(
        used <= 8,
        "{}: material params use {} f32 slots (max 8)",
        m.stem,
        used
    );

    let mut seen = HashSet::<&str>::new();
    for p in &m.header.params {
        if !seen.insert(p.ident.as_str()) {
            panic!("{}: duplicate param `{}`", m.stem, p.ident);
        }
    }
}

fn emit_rust_enum(mats: &[ParsedMaterial]) -> String {
    let mut s = String::new();
    s.push_str(
        "#[derive(Clone, Copy, Debug, PartialEq)]\n\
         pub enum Material {\n",
    );
    for m in mats {
        if m.header.params.is_empty() {
            s.push_str(&format!("    {},\n", m.variant));
        } else {
            s.push_str(&format!("    {} {{\n", m.variant));
            for p in &m.header.params {
                s.push_str(&format!(
                    "        {}: {},\n",
                    p.ident,
                    rust_field_type(&p.ty)
                ));
            }
            s.push_str("    },\n");
        }
    }
    s.push_str("}\n");
    s
}

fn emit_tag_impl(mats: &[ParsedMaterial]) -> String {
    let mut s = String::new();
    s.push_str("impl Material {\n");
    s.push_str("    pub fn tag(&self) -> u32 {\n        match self {\n");
    for (i, m) in mats.iter().enumerate() {
        if m.header.params.is_empty() {
            s.push_str(&format!(
                "            Material::{} => {},\n",
                m.variant, i
            ));
        } else {
            s.push_str(&format!(
                "            Material::{} {{ .. }} => {},\n",
                m.variant, i
            ));
        }
    }
    s.push_str("        }\n    }\n\n");

    s.push_str(
        "    /// Per-glyph material params packed into two `vec4` vertex attributes.\n\
         pub fn pack_for_vertex(&self) -> (u32, [f32; 4], [f32; 4]) {\n\
         let mut p = [0.0f32; 8];\n\
         match *self {\n",
    );

    for m in mats {
        if m.header.params.is_empty() {
            s.push_str(&format!("            Material::{} => {{ }}\n", m.variant));
        } else {
            let pat = format!(
                "Material::{} {{ {} }}",
                m.variant,
                m.header
                    .params
                    .iter()
                    .map(|x| x.ident.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            s.push_str(&format!("            {} => {{\n", pat));
            let mut idx = 0usize;
            for p in &m.header.params {
                match p.ty.trim() {
                    "f32" => {
                        s.push_str(&format!("                p[{}] = {};\n", idx, p.ident));
                        idx += 1;
                    }
                    "vec2<f32>" => {
                        s.push_str(&format!("                p[{}] = {}[0];\n", idx, p.ident));
                        s.push_str(&format!("                p[{}] = {}[1];\n", idx + 1, p.ident));
                        idx += 2;
                    }
                    "vec4<f32>" => {
                        for j in 0..4 {
                            s.push_str(&format!(
                                "                p[{}] = {}[{}];\n",
                                idx + j,
                                p.ident,
                                j
                            ));
                        }
                        idx += 4;
                    }
                    _ => unreachable!(),
                }
            }
            s.push_str("            }\n");
        }
    }

    s.push_str(
        "        }\n\
         let m0 = [p[0], p[1], p[2], p[3]];\n\
         let m1 = [p[4], p[5], p[6], p[7]];\n\
         (self.tag(), m0, m1)\n\
    }\n}\n",
    );
    s
}

fn emit_wgsl_dispatch(mats: &[ParsedMaterial]) -> String {
    let mut out = String::new();
    out.push_str(
        "struct MaterialParams {\n\
    p0: f32, p1: f32, p2: f32, p3: f32,\n\
    p4: f32, p5: f32, p6: f32, p7: f32,\n\
}\n\n",
    );

    for m in mats {
        out.push_str(&m.body);
        out.push_str("\n\n");
    }

    out.push_str(
        "fn dispatch_material(\n\
    tag: u32,\n\
    screen_px_range: f32,\n\
    sd_med: f32,\n\
    sd: f32,\n\
    sd_alpha: f32,\n\
    base_color: vec4<f32>,\n\
    p: MaterialParams,\n\
) -> vec4<f32> {\n\
    switch tag {\n",
    );

    for (i, m) in mats.iter().enumerate() {
        let fn_name = expected_fn(&m.stem);
        out.push_str(&format!(
            "        case {}u: {{\n            return {}(screen_px_range, sd_med, sd, sd_alpha, base_color, p);\n        }}\n",
            i, fn_name
        ));
    }

    out.push_str(
        "        default: {\n\
            return vec4<f32>(1.0, 0.0, 1.0, 1.0);\n\
        }\n\
    }\n\
}\n",
    );

    out
}

fn substitute_pixel_template(template: &str, dispatch: &str) -> String {
    const NEEDLE: &str = "// {{include generated/ubershader_dispatch.wgsl}}";
    if !template.contains(NEEDLE) {
        panic!("pixel.wgsl missing include placeholder {:?}", NEEDLE);
    }
    template.replace(NEEDLE, dispatch)
}

fn validate_wgsl(full_pixel: &str) {
    let module = naga::front::wgsl::parse_str(full_pixel).unwrap_or_else(|e| {
        panic!("naga WGSL parse failed:\n{e}\n---\n{full_pixel}\n---\n");
    });
    let mut val = Validator::new(ValidationFlags::all(), Capabilities::default());
    val.validate(&module)
        .unwrap_or_else(|e| panic!("naga validation failed:\n{e:?}\n"));
}

pub fn run(manifest_dir: &Path, out_dir: &Path) {
    let materials_dir = manifest_dir.join("materials");
    let paths = collect_material_wgsl(&materials_dir);
    for p in &paths {
        let rel = p.strip_prefix(manifest_dir).unwrap_or(p);
        println!("cargo:rerun-if-changed={}", rel.display());
    }
    let mats: Vec<ParsedMaterial> = paths.iter().map(|p| load_material(p)).collect();
    for m in &mats {
        validate_header(m);
    }

    let rust_out = out_dir.join("materials_codegen.rs");
    let mut rust = String::new();
    rust.push_str("// @generated by build.rs (MSDF materials)\n");
    rust.push_str(&emit_rust_enum(&mats));
    rust.push_str(&emit_tag_impl(&mats));
    fs::write(&rust_out, &rust).expect("write materials_codegen.rs");

    let dispatch = emit_wgsl_dispatch(&mats);
    let dispatch_path = out_dir.join("ubershader_dispatch.wgsl");
    fs::write(&dispatch_path, &dispatch).expect("write ubershader_dispatch.wgsl");

    let pixel_template_path = manifest_dir.join("src/shaders/pixel.wgsl");
    let pixel_template = fs::read_to_string(&pixel_template_path).expect("read pixel.wgsl");
    let pixel_full = substitute_pixel_template(&pixel_template, &dispatch);
    let pixel_full_path = out_dir.join("pixel_full.wgsl");
    fs::write(&pixel_full_path, &pixel_full).expect("write pixel_full.wgsl");

    validate_wgsl(&pixel_full);
}
