//! Naive whitespace-based line breaking.
//!
//! Copied from `text` crate (Slug renderer).

use crate::font::Font;

#[derive(Clone, Debug)]
pub(crate) struct Line {
    pub text: String,
    pub advance: f32,
}

pub(crate) fn break_lines(font: &Font, text: &str, max_width_em: f32) -> Vec<Line> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        wrap_paragraph(font, paragraph, max_width_em, &mut lines);
    }
    lines
}

fn wrap_paragraph(font: &Font, text: &str, max_width: f32, out: &mut Vec<Line>) {
    if text.is_empty() {
        out.push(Line {
            text: String::new(),
            advance: 0.0,
        });
        return;
    }

    let mut current = String::new();
    let mut current_w = 0.0f32;

    let mut tokens = tokenize(text);
    while let Some(tok) = tokens.next() {
        let tw = measure(font, tok);
        let is_ws = tok.chars().all(|c| c.is_whitespace());

        if current.is_empty() {
            if is_ws {
                continue;
            }
            current.push_str(tok);
            current_w = tw;
        } else if current_w + tw <= max_width {
            current.push_str(tok);
            current_w += tw;
        } else {
            out.push(Line {
                text: std::mem::take(&mut current),
                advance: current_w,
            });
            current_w = 0.0;
            if !is_ws {
                current.push_str(tok);
                current_w = tw;
            }
        }
    }

    if !current.is_empty() || out.is_empty() {
        out.push(Line {
            text: current,
            advance: current_w,
        });
    }
}

fn tokenize(text: &str) -> impl Iterator<Item = &str> {
    let mut start = 0;
    let mut prev_ws: Option<bool> = None;
    let bytes = text.as_bytes();
    std::iter::from_fn(move || {
        if start >= bytes.len() {
            return None;
        }
        let mut i = start;
        while i < bytes.len() {
            let c_start = i;
            let ch = text[c_start..].chars().next().unwrap();
            let ws = ch.is_whitespace();
            match prev_ws {
                None => prev_ws = Some(ws),
                Some(p) if p != ws => {
                    let s = &text[start..c_start];
                    start = c_start;
                    prev_ws = Some(ws);
                    return Some(s);
                }
                _ => {}
            }
            i += ch.len_utf8();
        }
        let s = &text[start..];
        start = bytes.len();
        prev_ws = None;
        Some(s)
    })
}

pub(crate) fn measure(font: &Font, text: &str) -> f32 {
    let mut total = 0.0;
    for ch in text.chars() {
        if let Some(gid) = font.glyph_index(ch) {
            total += font.advance_em(gid);
        } else if let Some(gid) = font.glyph_index('\u{FFFD}') {
            total += font.advance_em(gid);
        } else if ch == ' ' {
            total += 0.5;
        }
    }
    total
}
