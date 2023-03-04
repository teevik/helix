use std::cmp::{min, Ordering};
use std::convert::identity;

use helix_core::doc_formatter::{DocumentFormatter, GraphemeSource, TextFormat};
use helix_core::graphemes::Grapheme;
use helix_core::str_utils::char_to_byte_idx;
use helix_core::syntax::Highlight;
use helix_core::syntax::HighlightEvent;
use helix_core::text_annotations::TextAnnotations;
use helix_core::{visual_offset_from_block, Position, RopeSlice};
use helix_view::editor::{CursorCache, WhitespaceConfig, WhitespaceRenderValue};
use helix_view::graphics::Rect;
use helix_view::theme::Style;
use helix_view::view::ViewPosition;
use helix_view::{Document, Theme};
use tui::buffer::Buffer as Surface;

/// Decorations are the primary mechanisim for extending the text rendering.
///
/// Any on-screen element which is anchored to the rendered text in some form should
/// be implemented using this trait. Translating char positions to
/// on-screen positions can be expensive and should not be done during rendering.
/// Instead such translations are performed on the fly while the text is being rendered.
/// The results are provided to this trait
///
/// To reserve space for virtual text lines (which is then filled by this trait) emit appropriate
/// [`LineAnnotation`](helix_core::text_annotations::LineAnnotation) in
/// [`helix_view::Document::text_annotations`] or [`helix_view::View::text_annotations`]
pub trait Decoration {
    /// Called **before** a **visual** line is rendered. A visual line does not
    /// necessairly correspond to a single line in a document as soft wrapping can
    /// spread a single document line across multiple visual lines.
    ///
    /// This function is called before text is rendered as any decorations should
    /// never overlap the document text. That means that setting the forground color
    /// here is (essentially) useless as the text color is overwritten by the
    /// rendered text. This -ofcourse- doesn't apply when rendering inside virtual lines
    /// below the line reserved by `LineAnnotation`s. e as no text will be rendered here.
    fn decorate_line(&mut self, _renderer: &mut TextRenderer, _pos: LinePos) {}

    /// Called **after** a **visual** line is rendered. A visual line does not
    /// necessairly correspond to a single line in a document as soft wrapping can
    /// spread a single document line across multiple visual lines.
    ///
    /// This function is called after text is rendered so that decorations can collect
    /// horizontal positions on the line (see [`Decoration::render_position`]) first and
    /// use those positions` while rendering
    /// virtual text.
    /// That means that setting the forground color
    /// here is (essentially) useless as the text color is overwritten by the
    /// rendered text. This -ofcourse- doesn't apply when rendering inside virtual lines
    /// below the line reserved by `LineAnnotation`s. e as no text will be rendered here.
    /// **Note**: To avoid overlapping decorations in the virtual lines, each decoration
    /// must return the number of virtual text lines it has taken up. Each `Decoration` recieves
    /// an offset `virt_off` based on these return values where it can render virtual text:
    ///
    /// That means that a `render_line` implementation that returns `X` can render virtual text
    /// in the following area:
    /// ``` rust
    /// let start = inner.y + pos.virtual_line + virt_off;
    /// start .. start + X
    /// ````
    fn render_virt_lines(
        &mut self,
        _renderer: &mut TextRenderer,
        _pos: LinePos,
        _virt_off: usize,
    ) -> usize {
        0
    }

    /// This function is called **before** the grapheme at `char_idx` is rendered.
    /// A decoration must register all char indecies it is interested in (hence for which
    /// this funciton will be called) with [`DecorationsManager::register_positiom`].
    fn decorate_position(
        &mut self,
        _renderer: &mut TextRenderer,
        _char_idx: usize,
        _pos: Position,
    ) {
    }
}

impl<F: FnMut(&mut TextRenderer, LinePos)> Decoration for F {
    fn decorate_line(&mut self, renderer: &mut TextRenderer, pos: LinePos) {
        self(renderer, pos);
    }
}
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct DecorationRenderIdx(u32);

#[derive(Default)]
pub struct DecorationManager<'a> {
    position_hooks: Vec<(usize, DecorationRenderIdx)>,
    current_idx: usize,
    decorations: Vec<Box<dyn Decoration + 'a>>,
}

impl<'a> DecorationManager<'a> {
    pub fn add_decoration(&mut self, decoration: impl Decoration + 'a) -> DecorationRenderIdx {
        let idx = self.decorations.len() as u32;
        self.decorations.push(Box::new(decoration));
        DecorationRenderIdx(idx)
    }

    /// Register `char_idx` with the decoration manager so that `[Decoration::render_position]`
    /// is called when that `char_idx` is called for a decoration when `char_idx` is reached.
    ///
    /// The `char_idx` don't need to be registered in order as they are sorted anyway.
    /// However sorting is (slightly) faster if they are. If the same `char_idx` multiple times
    /// for the same decoration  then
    pub fn register_positon(&mut self, decoration: DecorationRenderIdx, char_idx: usize) {
        self.position_hooks.push((char_idx, decoration))
    }

    fn prepare_for_rendering(&mut self, first_visible_char: usize) {
        // Sort by char index, if the char index is identical, sort by the `DecorationRenderIdx`
        // so that decorations are called in the order they were added
        self.position_hooks.sort_unstable();
        self.current_idx = self
            .position_hooks
            .binary_search_by_key(&first_visible_char, |&(char_pos, _)| char_pos)
            .unwrap_or_else(identity);
    }

    fn decorate_position(&mut self, char_idx: usize, renderer: &mut TextRenderer, pos: Position) {
        for &(hook_char_idx, decoration) in &self.position_hooks[self.current_idx..] {
            match hook_char_idx.cmp(&char_idx) {
                // this grapheme has been concealed by a fold etc.
                // (currently unimplemented, but considered here for future proofing)
                Ordering::Less => (),
                Ordering::Equal => self.decorations[decoration.0 as usize]
                    .decorate_position(renderer, char_idx, pos),
                Ordering::Greater => break,
            }

            self.current_idx += 1;
        }
    }

    fn decorate_line(&mut self, renderer: &mut TextRenderer, pos: LinePos) {
        for decoration in &mut self.decorations {
            decoration.decorate_line(renderer, pos);
        }
    }

    fn render_virtual_lines(&mut self, renderer: &mut TextRenderer, pos: LinePos) {
        let mut virt_off = 0;
        for decoration in &mut self.decorations {
            virt_off += decoration.render_virt_lines(renderer, pos, virt_off);
        }
    }
}

impl<'a> Decoration for &'a CursorCache {
    fn decorate_position(&mut self, _renderer: &mut TextRenderer, _char_idx: usize, pos: Position) {
        self.set(Some(pos))
    }
}

/// A wrapper around a HighlightIterator
/// that merges the layered highlights to create the final text style
/// and yields the active text style and the char_idx where the active
/// style will have to be recomputed.
struct StyleIter<'a, H: Iterator<Item = HighlightEvent>> {
    text_style: Style,
    active_highlights: Vec<Highlight>,
    highlight_iter: H,
    theme: &'a Theme,
}

impl<H: Iterator<Item = HighlightEvent>> Iterator for StyleIter<'_, H> {
    type Item = (Style, usize);
    fn next(&mut self) -> Option<(Style, usize)> {
        while let Some(event) = self.highlight_iter.next() {
            match event {
                HighlightEvent::HighlightStart(highlights) => {
                    self.active_highlights.push(highlights)
                }
                HighlightEvent::HighlightEnd => {
                    self.active_highlights.pop();
                }
                HighlightEvent::Source { start, end } => {
                    if start == end {
                        continue;
                    }
                    let style = self
                        .active_highlights
                        .iter()
                        .fold(self.text_style, |acc, span| {
                            acc.patch(self.theme.highlight(span.0))
                        });
                    return Some((style, end));
                }
            }
        }
        None
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct LinePos {
    /// Indicates whether the given visual line
    /// is the first visual line of the given document line
    pub first_visual_line: bool,
    /// The line index of the document line that contains the given visual line
    pub doc_line: usize,
    /// Vertical offset from the top of the inner view area
    pub visual_line: u16,
}

#[allow(clippy::too_many_arguments)]
pub fn render_document(
    surface: &mut Surface,
    viewport: Rect,
    doc: &Document,
    offset: ViewPosition,
    doc_annotations: &TextAnnotations,
    highlight_iter: impl Iterator<Item = HighlightEvent>,
    theme: &Theme,
    decorations: DecorationManager,
) {
    let mut renderer = TextRenderer::new(surface, doc, theme, offset.horizontal_offset, viewport);
    render_text(
        &mut renderer,
        doc.text().slice(..),
        offset,
        &doc.text_format(viewport.width, Some(theme)),
        doc_annotations,
        highlight_iter,
        theme,
        decorations,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn render_text<'t>(
    renderer: &mut TextRenderer,
    text: RopeSlice<'t>,
    offset: ViewPosition,
    text_fmt: &TextFormat,
    text_annotations: &TextAnnotations,
    highlight_iter: impl Iterator<Item = HighlightEvent>,
    theme: &Theme,
    mut decorations: DecorationManager,
) {
    let (
        Position {
            row: mut row_off, ..
        },
        mut char_pos,
    ) = visual_offset_from_block(
        text,
        offset.anchor,
        offset.anchor,
        text_fmt,
        text_annotations,
    );
    row_off += offset.vertical_offset;
    assert_eq!(0, offset.vertical_offset);

    let (mut formatter, char_off) =
        DocumentFormatter::new_at_prev_checkpoint(text, text_fmt, text_annotations, offset.anchor);
    let mut styles = StyleIter {
        text_style: renderer.text_style,
        active_highlights: Vec::with_capacity(64),
        highlight_iter,
        theme,
    };
    decorations.prepare_for_rendering(char_off);

    let mut last_line_pos = LinePos {
        first_visual_line: false,
        doc_line: usize::MAX,
        visual_line: u16::MAX,
    };
    let mut is_in_indent_area = true;
    let mut last_line_indent_level = 0;
    let mut style_span = styles
        .next()
        .unwrap_or_else(|| (Style::default(), usize::MAX));

    loop {
        // formattter.line_pos returns to line index of the next grapheme
        // so it must be called before formatter.next
        let doc_line = formatter.line_pos();
        // TODO refactor with let .. else once MSRV reaches 1.65
        let (grapheme, mut pos) = if let Some(it) = formatter.next() {
            it
        } else {
            let mut last_pos = formatter.visual_pos();
            if last_pos.row >= row_off {
                last_pos.col -= 1;
                last_pos.row -= row_off;
                // decorate EOF char
                decorations.decorate_position(char_pos, renderer, last_pos);
            }
            break;
        };

        // skip any graphemes on visual lines before the block start
        if pos.row < row_off {
            if char_pos >= style_span.1 {
                // TODO refactor using let..else once MSRV reaches 1.65
                style_span = if let Some(style_span) = styles.next() {
                    style_span
                } else {
                    break;
                }
            }
            char_pos += grapheme.doc_chars();
            continue;
        }
        pos.row -= row_off;

        // if the end of the viewport is reached stop rendering
        if pos.row as u16 >= renderer.viewport.height {
            break;
        }

        // apply decorations before rendering a new line
        if pos.row as u16 != last_line_pos.visual_line {
            if pos.row > 0 {
                // draw indent guides for the last line
                renderer.draw_indent_guides(last_line_indent_level, last_line_pos.visual_line);
                is_in_indent_area = true;
                decorations.render_virtual_lines(renderer, last_line_pos)
            }
            last_line_pos = LinePos {
                first_visual_line: doc_line != last_line_pos.doc_line,
                doc_line,
                visual_line: pos.row as u16,
            };
            decorations.decorate_line(renderer, last_line_pos);
        }

        // aquire the correct grapheme style
        if char_pos >= style_span.1 {
            // TODO refactor using let..else once MSRV reaches 1.65
            style_span = if let Some(style_span) = styles.next() {
                style_span
            } else {
                (Style::default(), usize::MAX)
            }
        }

        let grapheme_style = if let GraphemeSource::VirtualText { highlight } = grapheme.source {
            let style = renderer.text_style;
            if let Some(highlight) = highlight {
                style.patch(theme.highlight(highlight.0))
            } else {
                style
            }
        } else {
            style_span.0
        };
        decorations.decorate_position(char_pos, renderer, pos);
        char_pos += grapheme.doc_chars();

        renderer.draw_grapheme(
            grapheme.grapheme,
            grapheme_style,
            &mut last_line_indent_level,
            &mut is_in_indent_area,
            pos,
        );
    }

    renderer.draw_indent_guides(last_line_indent_level, last_line_pos.visual_line);
    decorations.render_virtual_lines(renderer, last_line_pos)
}

#[derive(Debug)]
pub struct TextRenderer<'a> {
    pub surface: &'a mut Surface,
    pub text_style: Style,
    pub whitespace_style: Style,
    pub indent_guide_char: String,
    pub indent_guide_style: Style,
    pub newline: String,
    pub nbsp: String,
    pub space: String,
    pub tab: String,
    pub tab_width: u16,
    pub starting_indent: usize,
    pub draw_indent_guides: bool,
    pub col_offset: usize,
    pub viewport: Rect,
}

impl<'a> TextRenderer<'a> {
    pub fn new(
        surface: &'a mut Surface,
        doc: &Document,
        theme: &Theme,
        col_offset: usize,
        viewport: Rect,
    ) -> TextRenderer<'a> {
        let editor_config = doc.config.load();
        let WhitespaceConfig {
            render: ws_render,
            characters: ws_chars,
        } = &editor_config.whitespace;

        let tab_width = doc.tab_width();
        let tab = if ws_render.tab() == WhitespaceRenderValue::All {
            std::iter::once(ws_chars.tab)
                .chain(std::iter::repeat(ws_chars.tabpad).take(tab_width - 1))
                .collect()
        } else {
            " ".repeat(tab_width)
        };
        let newline = if ws_render.newline() == WhitespaceRenderValue::All {
            ws_chars.newline.into()
        } else {
            " ".to_owned()
        };

        let space = if ws_render.space() == WhitespaceRenderValue::All {
            ws_chars.space.into()
        } else {
            " ".to_owned()
        };
        let nbsp = if ws_render.nbsp() == WhitespaceRenderValue::All {
            ws_chars.nbsp.into()
        } else {
            " ".to_owned()
        };

        let text_style = theme.get("ui.text");

        TextRenderer {
            surface,
            indent_guide_char: editor_config.indent_guides.character.into(),
            newline,
            nbsp,
            space,
            tab_width: tab_width as u16,
            tab,
            whitespace_style: theme.get("ui.virtual.whitespace"),
            starting_indent: (col_offset / tab_width)
                + editor_config.indent_guides.skip_levels as usize,
            indent_guide_style: text_style.patch(
                theme
                    .try_get("ui.virtual.indent-guide")
                    .unwrap_or_else(|| theme.get("ui.virtual.whitespace")),
            ),
            text_style,
            draw_indent_guides: editor_config.indent_guides.render,
            viewport,
            col_offset,
        }
    }

    /// Draws a single `grapheme` at the current render position with a specified `style`.
    pub fn draw_grapheme(
        &mut self,
        grapheme: Grapheme,
        mut style: Style,
        last_indent_level: &mut usize,
        is_in_indent_area: &mut bool,
        position: Position,
    ) {
        let cut_off_start = self.col_offset.saturating_sub(position.col);
        let is_whitespace = grapheme.is_whitespace();

        // TODO is it correct to apply the whitspace style to all unicode white spaces?
        if is_whitespace {
            style = style.patch(self.whitespace_style);
        }

        let width = grapheme.width();
        let grapheme = match grapheme {
            Grapheme::Tab { width } => {
                let grapheme_tab_width = char_to_byte_idx(&self.tab, width);
                &self.tab[..grapheme_tab_width]
            }
            // TODO special rendering for other whitespaces?
            Grapheme::Other { ref g } if g == " " => &self.space,
            Grapheme::Other { ref g } if g == "\u{00A0}" => &self.nbsp,
            Grapheme::Other { ref g } => g,
            Grapheme::Newline => &self.newline,
        };

        let in_bounds = self.col_offset <= position.col
            && position.col < self.viewport.width as usize + self.col_offset;

        if in_bounds {
            self.surface.set_string(
                self.viewport.x + (position.col - self.col_offset) as u16,
                self.viewport.y + position.row as u16,
                grapheme,
                style,
            );
        } else if cut_off_start != 0 && cut_off_start < width {
            // partially on screen
            let rect = Rect::new(
                self.viewport.x,
                self.viewport.y + position.row as u16,
                (width - cut_off_start) as u16,
                1,
            );
            self.surface.set_style(rect, style);
        }

        if *is_in_indent_area && !is_whitespace {
            *last_indent_level = position.col;
            *is_in_indent_area = false;
        }
    }

    /// Overlay indentation guides ontop of a rendered line
    /// The indentation level is computed in `draw_lines`.
    /// Therefore this function must always be called afterwards.
    pub fn draw_indent_guides(&mut self, indent_level: usize, row: u16) {
        if !self.draw_indent_guides {
            return;
        }

        // Don't draw indent guides outside of view
        let end_indent = min(
            indent_level,
            // Add tab_width - 1 to round up, since the first visible
            // indent might be a bit after offset.col
            self.col_offset + self.viewport.width as usize + (self.tab_width - 1) as usize,
        ) / self.tab_width as usize;

        for i in self.starting_indent..end_indent {
            let x =
                (self.viewport.x as usize + (i * self.tab_width as usize) - self.col_offset) as u16;
            let y = self.viewport.y + row;
            debug_assert!(self.surface.in_bounds(x, y));
            self.surface
                .set_string(x, y, &self.indent_guide_char, self.indent_guide_style);
        }
    }
}
