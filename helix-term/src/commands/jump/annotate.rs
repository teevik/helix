use std::rc::Rc;

use super::JumpAnnotation;
use crate::commands::Context;
use helix_core::text_annotations::Overlay;
use helix_view::input::KeyEvent;

pub const JUMP_KEYS: &str = "etovxqpdygfblzhckisuran";

pub fn apply_dimming(_ctx: &mut Context) {
    // TODO solve with a highlight iterator
    // layer, you are not changing the on-screen text
    // so this does not require virtual text at all

    // let (view, doc) = current!(ctx.editor);
    // let first_line = view.offset.row;
    // let num_lines = view.last_line(doc) - first_line + 1;

    // let lines: Vec<_> = doc
    //     .text()
    //     .lines_at(first_line)
    //     .zip(first_line..)
    //     .take(num_lines)
    //     .map(|(line, idx)| TextAnnotation {
    //         text: String::from(line).into(),
    //         style: Style::default().fg(Color::Rgb(0x66, 0x66, 0x66)),
    //         line: idx,
    //         kind: TextAnnotationKind::Overlay(0),
    //     })
    //     .collect();
    // doc.push_text_annotations("jump_mode", lines.into_iter());
}

pub fn clear_dimming(ctx: &mut Context) {
    view_mut!(ctx.editor).jump_labels = Rc::new([]);
}

pub fn show_key_annotations_with_callback<F>(
    ctx: &mut Context,
    annotations: Vec<JumpAnnotation>,
    on_key_press_callback: F,
) where
    F: FnOnce(&mut Context, KeyEvent) + 'static,
{
    apply_dimming(ctx);
    // TODO: create seperate highlight layers
    // let style = match jump.keys.len() {
    //     2.. => multi_first_style,
    //     _ => single_style,
    // };
    let mut overlays: Rc<[_]> = annotations
        .iter()
        .flat_map(|jump| {
            jump.keys.iter().enumerate().map(move |(i, c)| Overlay {
                char_idx: jump.loc + i,
                grapheme: c.to_string().into(),
            })
        })
        .collect();
    // TODO do we really need
    Rc::get_mut(&mut overlays)
        .unwrap()
        .sort_unstable_by_key(|overlay| overlay.char_idx);
    view_mut!(ctx.editor).jump_labels = overlays;
    ctx.on_next_key(on_key_press_callback);
}
