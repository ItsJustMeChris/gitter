//! Single- and multi-line text input with selection, IME, clipboard, undo,
//! soft wrapping and scrolling. Built on GPUI's `EntityInputHandler`.

use crate::theme::{mono_font, ActiveTheme};
use gpui::{
    actions, div, fill, point, prelude::*, px, relative, size, App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, Hsla, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, ScrollWheelEvent, SharedString, Style, TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, WrappedLine,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteWordRight,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectAll,
        Home,
        End,
        SelectHome,
        SelectEnd,
        DocStart,
        DocEnd,
        Paste,
        Cut,
        Copy,
        Undo,
        Redo,
        Enter,
        SecondaryEnter,
        Tab,
        ShiftTab,
    ]
);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("TextInput");
    let word = if cfg!(target_os = "macos") { "alt" } else { "ctrl" };
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("shift-backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new(&format!("{word}-backspace"), DeleteWordLeft, ctx),
        KeyBinding::new(&format!("{word}-delete"), DeleteWordRight, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("up", Up, ctx),
        KeyBinding::new("down", Down, ctx),
        KeyBinding::new(&format!("{word}-left"), WordLeft, ctx),
        KeyBinding::new(&format!("{word}-right"), WordRight, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("shift-up", SelectUp, ctx),
        KeyBinding::new("shift-down", SelectDown, ctx),
        KeyBinding::new(&format!("{word}-shift-left"), SelectWordLeft, ctx),
        KeyBinding::new(&format!("{word}-shift-right"), SelectWordRight, ctx),
        KeyBinding::new("secondary-a", SelectAll, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("shift-home", SelectHome, ctx),
        KeyBinding::new("shift-end", SelectEnd, ctx),
        KeyBinding::new("secondary-home", DocStart, ctx),
        KeyBinding::new("secondary-end", DocEnd, ctx),
        KeyBinding::new("secondary-v", Paste, ctx),
        KeyBinding::new("shift-insert", Paste, ctx),
        KeyBinding::new("secondary-c", Copy, ctx),
        KeyBinding::new("secondary-insert", Copy, ctx),
        KeyBinding::new("secondary-x", Cut, ctx),
        KeyBinding::new("secondary-z", Undo, ctx),
        KeyBinding::new("secondary-shift-z", Redo, ctx),
        KeyBinding::new("secondary-y", Redo, ctx),
        KeyBinding::new("enter", Enter, ctx),
        KeyBinding::new("secondary-enter", SecondaryEnter, ctx),
        KeyBinding::new("tab", Tab, ctx),
        KeyBinding::new("shift-tab", ShiftTab, ctx),
    ]);
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-left", Home, ctx),
        KeyBinding::new("cmd-right", End, ctx),
        KeyBinding::new("cmd-shift-left", SelectHome, ctx),
        KeyBinding::new("cmd-shift-right", SelectEnd, ctx),
        KeyBinding::new("cmd-up", DocStart, ctx),
        KeyBinding::new("cmd-down", DocEnd, ctx),
        KeyBinding::new("cmd-backspace", DeleteWordLeft, ctx),
    ]);
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    Changed,
    /// Enter in a single-line input, or Ctrl/Cmd+Enter in a multi-line one.
    Submit,
    Focused,
    Blurred,
}

#[derive(Clone)]
struct Snapshot {
    content: String,
    selection: Range<usize>,
}

/// Layout of the most recent paint, used for hit testing and caret movement.
struct InputLayout {
    lines: Vec<LaidLine>,
    line_height: Pixels,
    content_height: Pixels,
    content_width: Pixels,
}

struct LaidLine {
    start: usize,
    y: Pixels,
    line: WrappedLine,
}

impl InputLayout {
    fn line_for_offset(&self, offset: usize) -> Option<&LaidLine> {
        self.lines.iter().rev().find(|l| l.start <= offset).or(self.lines.first())
    }

    fn position_for_offset(&self, offset: usize) -> Point<Pixels> {
        let Some(l) = self.line_for_offset(offset) else { return point(px(0.), px(0.)) };
        let local = offset.saturating_sub(l.start).min(l.line.len());
        let p = l.line.position_for_index(local, self.line_height).unwrap_or(point(px(0.), px(0.)));
        point(p.x, p.y + l.y)
    }

    fn offset_for_position(&self, p: Point<Pixels>) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        let mut target = &self.lines[0];
        for l in &self.lines {
            if p.y >= l.y {
                target = l;
            }
        }
        let local_y = (p.y - target.y).max(px(0.)).min(target.line.size(self.line_height).height - px(1.));
        let ix = match target.line.closest_index_for_position(point(p.x.max(px(0.)), local_y), self.line_height) {
            Ok(i) | Err(i) => i,
        };
        target.start + ix.min(target.line.len())
    }

    /// Byte ranges of every visual (wrapped) row with its y position.
    fn visual_rows(&self) -> Vec<(Range<usize>, Pixels)> {
        let mut rows = Vec::new();
        for l in &self.lines {
            let mut row_start = 0;
            for (i, b) in l.line.wrap_boundaries().iter().enumerate() {
                let ix = l.line.runs()[b.run_ix].glyphs[b.glyph_ix].index;
                rows.push((l.start + row_start..l.start + ix, l.y + self.line_height * i as f32));
                row_start = ix;
            }
            let n = l.line.wrap_boundaries().len();
            rows.push((l.start + row_start..l.start + l.line.len(), l.y + self.line_height * n as f32));
        }
        rows
    }
}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: String,
    placeholder: SharedString,
    multiline: bool,
    monospace: bool,
    bordered: bool,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    layout: Option<InputLayout>,
    bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    scroll: Point<Pixels>,
    scroll_to_cursor: bool,
    goal_x: Option<Pixels>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit_was_typing: bool,
    was_focused: bool,
}

impl EventEmitter<InputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle().tab_stop(true),
            content: String::new(),
            placeholder: SharedString::default(),
            multiline: false,
            monospace: false,
            bordered: true,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            layout: None,
            bounds: None,
            is_selecting: false,
            scroll: point(px(0.), px(0.)),
            scroll_to_cursor: false,
            goal_x: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit_was_typing: false,
            was_focused: false,
        }
    }

    pub fn multiline(mut self) -> Self {
        self.multiline = true;
        self
    }

    pub fn with_placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = text.into();
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.content = self.sanitize(text.into());
        self.selected_range = self.content.len()..self.content.len();
        self
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.content = self.sanitize(text.into());
        self.selected_range = self.content.len()..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        self.undo.clear();
        self.redo.clear();
        self.scroll = point(px(0.), px(0.));
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_text("", cx);
    }

    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn sanitize(&self, text: String) -> String {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.multiline {
            text
        } else {
            text.replace('\n', " ")
        }
    }

    // ------------------------------------------------------------------
    // Editing primitives
    // ------------------------------------------------------------------

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.scroll_to_cursor = true;
        self.last_edit_was_typing = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.scroll_to_cursor = true;
        self.last_edit_was_typing = false;
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).rev().find_map(|(idx, _)| (idx < offset).then_some(idx)).unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).find_map(|(idx, _)| (idx > offset).then_some(idx)).unwrap_or(self.content.len())
    }

    fn previous_word(&self, offset: usize) -> usize {
        let mut last = 0;
        for (idx, word) in self.content.split_word_bound_indices() {
            if idx >= offset {
                break;
            }
            if !word.trim().is_empty() {
                last = idx;
            }
        }
        last
    }

    fn next_word(&self, offset: usize) -> usize {
        for (idx, word) in self.content.split_word_bound_indices() {
            let end = idx + word.len();
            if end > offset && !word.trim().is_empty() {
                return end;
            }
        }
        self.content.len()
    }

    fn word_range_at(&self, offset: usize) -> Range<usize> {
        for (idx, word) in self.content.split_word_bound_indices() {
            if offset >= idx && offset < idx + word.len() {
                return idx..idx + word.len();
            }
        }
        offset..offset
    }

    fn line_start(&self, offset: usize) -> usize {
        if let Some(layout) = &self.layout {
            for (range, _) in layout.visual_rows() {
                if offset >= range.start && offset <= range.end {
                    return range.start;
                }
            }
        }
        self.content[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0)
    }

    fn line_end(&self, offset: usize) -> usize {
        if let Some(layout) = &self.layout {
            for (range, _) in layout.visual_rows() {
                if offset >= range.start && offset <= range.end {
                    return range.end;
                }
            }
        }
        self.content[offset..].find('\n').map(|i| offset + i).unwrap_or(self.content.len())
    }

    fn vertical_target(&mut self, offset: usize, down: bool) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let pos = layout.position_for_offset(offset);
        let x = *self.goal_x.get_or_insert(pos.x);
        let y = if down { pos.y + layout.line_height * 1.5 } else { pos.y - layout.line_height * 0.5 };
        if y < px(0.) {
            return Some(0);
        }
        if y >= layout.content_height {
            return Some(self.content.len());
        }
        Some(layout.offset_for_position(point(x, y)))
    }

    fn push_undo(&mut self, typing: bool) {
        if typing && self.last_edit_was_typing {
            return;
        }
        self.undo.push(Snapshot { content: self.content.clone(), selection: self.selected_range.clone() });
        if self.undo.len() > 300 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let text = self.sanitize(text.to_string());
        let typing = text.chars().count() == 1 && !text.contains(char::is_whitespace) && range.is_empty();
        self.push_undo(typing);
        self.last_edit_was_typing = typing;
        self.content.replace_range(range.clone(), &text);
        let caret = range.start + text.len();
        self.selected_range = caret..caret;
        self.selection_reversed = false;
        self.marked_range = None;
        self.goal_x = None;
        self.scroll_to_cursor = true;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    // ------------------------------------------------------------------
    // Actions
    // ------------------------------------------------------------------

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let prev = self.previous_boundary(self.cursor_offset());
            self.replace(prev..self.cursor_offset(), "", cx);
        } else {
            self.replace(self.selected_range.clone(), "", cx);
        }
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = self.next_boundary(self.cursor_offset());
            self.replace(self.cursor_offset()..next, "", cx);
        } else {
            self.replace(self.selected_range.clone(), "", cx);
        }
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let prev = self.previous_word(self.cursor_offset());
            self.replace(prev..self.cursor_offset(), "", cx);
        } else {
            self.replace(self.selected_range.clone(), "", cx);
        }
    }

    fn delete_word_right(&mut self, _: &DeleteWordRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = self.next_word(self.cursor_offset());
            self.replace(self.cursor_offset()..next, "", cx);
        } else {
            self.replace(self.selected_range.clone(), "", cx);
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if !self.multiline {
            cx.propagate();
            return;
        }
        let goal = self.goal_x;
        if let Some(t) = self.vertical_target(self.cursor_offset(), false) {
            self.move_to(t, cx);
        }
        self.goal_x = goal.or(self.goal_x);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if !self.multiline {
            cx.propagate();
            return;
        }
        let goal = self.goal_x;
        if let Some(t) = self.vertical_target(self.cursor_offset(), true) {
            self.move_to(t, cx);
        }
        self.goal_x = goal.or(self.goal_x);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        if !self.multiline {
            self.select_to(0, cx);
            return;
        }
        if let Some(t) = self.vertical_target(self.cursor_offset(), false) {
            self.select_to(t, cx);
        }
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        if !self.multiline {
            self.select_to(self.content.len(), cx);
            return;
        }
        if let Some(t) = self.vertical_target(self.cursor_offset(), true) {
            self.select_to(t, cx);
        }
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.previous_word(self.cursor_offset()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.next_word(self.cursor_offset()), cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_word(self.cursor_offset()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_word(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.move_to(self.line_start(self.cursor_offset()), cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.move_to(self.line_end(self.cursor_offset()), cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line_start(self.cursor_offset()), cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line_end(self.cursor_offset()), cx);
    }

    fn doc_start(&mut self, _: &DocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn doc_end(&mut self, _: &DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.last_edit_was_typing = false;
            self.replace(self.selected_range.clone(), &text, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected_range.clone()].to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected_range.clone()].to_string()));
            self.replace(self.selected_range.clone(), "", cx);
        }
    }

    fn undo_action(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(Snapshot { content: self.content.clone(), selection: self.selected_range.clone() });
            self.content = s.content;
            self.selected_range = s.selection;
            self.last_edit_was_typing = false;
            self.scroll_to_cursor = true;
            cx.emit(InputEvent::Changed);
            cx.notify();
        }
    }

    fn redo_action(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(Snapshot { content: self.content.clone(), selection: self.selected_range.clone() });
            self.content = s.content;
            self.selected_range = s.selection;
            self.last_edit_was_typing = false;
            self.scroll_to_cursor = true;
            cx.emit(InputEvent::Changed);
            cx.notify();
        }
    }

    fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.last_edit_was_typing = false;
            self.replace(self.selected_range.clone(), "\n", cx);
        } else {
            cx.emit(InputEvent::Submit);
        }
    }

    fn secondary_enter(&mut self, _: &SecondaryEnter, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submit);
    }

    fn tab(&mut self, _: &Tab, window: &mut Window, _: &mut Context<Self>) {
        window.focus_next();
    }

    fn shift_tab(&mut self, _: &ShiftTab, window: &mut Window, _: &mut Context<Self>) {
        window.focus_prev();
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        let offset = self.index_for_mouse_position(event.position);
        self.goal_x = None;
        match event.click_count {
            2 => {
                let r = self.word_range_at(offset);
                self.selected_range = r;
                self.selection_reversed = false;
                cx.notify();
            }
            3 => {
                self.selected_range = self.line_start(offset)..self.line_end(offset);
                self.selection_reversed = false;
                cx.notify();
            }
            _ => {
                self.is_selecting = true;
                if event.modifiers.shift {
                    self.select_to(offset, cx);
                } else {
                    self.move_to(offset, cx)
                }
            }
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            let offset = self.index_for_mouse_position(event.position);
            self.select_to(offset, cx);
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(layout), Some(bounds)) = (self.layout.as_ref(), self.bounds) else { return };
        let delta = event.delta.pixel_delta(layout.line_height);
        if self.multiline {
            let max = (layout.content_height - bounds.size.height).max(px(0.));
            self.scroll.y = (self.scroll.y - delta.y).clamp(px(0.), max);
        } else {
            let max = (layout.content_width - bounds.size.width + px(2.)).max(px(0.));
            let d = if delta.x.abs() > delta.y.abs() { delta.x } else { delta.y };
            self.scroll.x = (self.scroll.x - d).clamp(px(0.), max);
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(layout)) = (self.bounds.as_ref(), self.layout.as_ref()) else { return 0 };
        if self.content.is_empty() {
            return 0;
        }
        let local = point(position.x - bounds.left() + self.scroll.x, position.y - bounds.top() + self.scroll.y);
        layout.offset_for_position(local)
    }

    // UTF-16 conversions for the platform input handler.

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(&mut self, range_utf16: Range<usize>, actual_range: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content.get(range)?.to_string())
    }

    fn selected_text_range(&mut self, _ignore_disabled_input: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range_to_utf16(&self.selected_range), reversed: self.selection_reversed })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, new_text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.as_ref().map(|r| self.range_from_utf16(r)).or(self.marked_range.clone()).unwrap_or(self.selected_range.clone());
        self.replace(range, new_text, cx);
    }

    fn replace_and_mark_text_in_range(&mut self, range_utf16: Option<Range<usize>>, new_text: &str, new_selected_range_utf16: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.as_ref().map(|r| self.range_from_utf16(r)).or(self.marked_range.clone()).unwrap_or(self.selected_range.clone());
        self.content.replace_range(range.clone(), new_text);
        self.marked_range = if new_text.is_empty() { None } else { Some(range.start..range.start + new_text.len()) };
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .map(|new_range| new_range.start + range.start..new_range.end + range.start)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.scroll_to_cursor = true;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(&mut self, range_utf16: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let start = layout.position_for_offset(range.start);
        let end = layout.position_for_offset(range.end);
        Some(Bounds::from_corners(
            point(bounds.left() + start.x - self.scroll.x, bounds.top() + start.y - self.scroll.y),
            point(bounds.left() + end.x - self.scroll.x, bounds.top() + end.y + layout.line_height - self.scroll.y),
        ))
    }

    fn character_index_for_point(&mut self, point: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let ix = self.index_for_mouse_position(point);
        Some(self.offset_to_utf16(ix))
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let focused = self.focus_handle.is_focused(window);
        if focused != self.was_focused {
            self.was_focused = focused;
            cx.emit(if focused { InputEvent::Focused } else { InputEvent::Blurred });
        }
        let mut base = div()
            .id("text-input")
            .key_context("TextInput")
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::doc_start))
            .on_action(cx.listener(Self::doc_end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::undo_action))
            .on_action(cx.listener(Self::redo_action))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::secondary_enter))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::shift_tab))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .w_full()
            .text_color(theme.text)
            .line_height(px(18.));
        if self.monospace {
            base = base.font_family(mono_font());
        }
        if self.multiline {
            base = base.h_full().py(px(6.));
        } else {
            base = base.h(px(28.)).flex().items_center();
        }
        if self.bordered {
            base = base
                .px(px(8.))
                .rounded(px(5.))
                .bg(theme.input_bg)
                .border_1()
                .border_color(if focused { theme.border_focus } else { theme.border });
        }
        base.child(TextElement { input: cx.entity() })
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    lines: Vec<(Point<Pixels>, WrappedLine)>,
    cursor: Option<PaintQuad>,
    selections: Vec<PaintQuad>,
    layout: Option<InputLayout>,
}

impl IntoElement for TextElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let multiline = self.input.read(cx).multiline;
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if multiline {
            style.size.height = relative(1.).into();
        } else {
            style.size.height = window.line_height().into();
        }
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) -> PrepaintState {
        let theme = cx.theme().clone();
        let input = self.input.read(cx);
        let content = input.content.clone();
        let multiline = input.multiline;
        let selected_range = input.selected_range.clone();
        let cursor_offset = input.cursor_offset();
        let marked = input.marked_range.clone();
        let mut scroll = input.scroll;
        let scroll_to_cursor = input.scroll_to_cursor;
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();

        let (display_text, color): (SharedString, Hsla) = if content.is_empty() { (input.placeholder.clone(), theme.text_faint) } else { (content.clone().into(), style.color) };

        let run = TextRun { len: display_text.len(), font: style.font(), color, background_color: None, underline: None, strikethrough: None };
        let runs = match marked.as_ref().filter(|_| !content.is_empty()) {
            Some(m) => vec![
                TextRun { len: m.start, ..run.clone() },
                TextRun { len: m.end - m.start, underline: Some(UnderlineStyle { color: Some(run.color), thickness: px(1.0), wavy: false }), ..run.clone() },
                TextRun { len: display_text.len() - m.end, ..run },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect(),
            None => vec![run],
        };

        let wrap = if multiline { Some(bounds.size.width.max(px(10.))) } else { None };
        let shaped = window.text_system().shape_text(display_text.clone(), font_size, &runs, wrap, None).unwrap_or_default();

        let mut laid = Vec::new();
        let mut y = px(0.);
        let mut start = 0;
        let mut max_width = px(0.);
        for line in shaped {
            let h = line.size(line_height).height;
            max_width = max_width.max(line.width());
            let len = line.len();
            laid.push(LaidLine { start, y, line });
            start += len + 1;
            y += h;
        }
        let layout = InputLayout { lines: laid, line_height, content_height: y.max(line_height), content_width: max_width };

        // Placeholder shouldn't be hit-tested as real content.
        let real_layout = !content.is_empty();
        let cursor_pos = if real_layout { layout.position_for_offset(cursor_offset) } else { point(px(0.), px(0.)) };

        if scroll_to_cursor {
            if multiline {
                if cursor_pos.y < scroll.y {
                    scroll.y = cursor_pos.y;
                } else if cursor_pos.y + line_height > scroll.y + bounds.size.height {
                    scroll.y = cursor_pos.y + line_height - bounds.size.height;
                }
            } else {
                let margin = px(2.);
                if cursor_pos.x < scroll.x {
                    scroll.x = cursor_pos.x;
                } else if cursor_pos.x + margin > scroll.x + bounds.size.width {
                    scroll.x = cursor_pos.x + margin - bounds.size.width;
                }
            }
        }
        if multiline {
            let max = (layout.content_height - bounds.size.height).max(px(0.));
            scroll.y = scroll.y.clamp(px(0.), max);
            scroll.x = px(0.);
        } else {
            let max = (layout.content_width - bounds.size.width + px(2.)).max(px(0.));
            scroll.x = scroll.x.clamp(px(0.), max);
            scroll.y = px(0.);
        }

        let origin = point(bounds.left() - scroll.x, bounds.top() - scroll.y);

        let mut selections = Vec::new();
        if real_layout && !selected_range.is_empty() {
            for (row, row_y) in layout.visual_rows() {
                let s = selected_range.start.max(row.start);
                let e = selected_range.end.min(row.end);
                let row_contains_newline_sel = selected_range.end > row.end && selected_range.start <= row.end;
                if s > e || (s == e && !row_contains_newline_sel) {
                    continue;
                }
                let x0 = if s <= row.start { px(0.) } else { layout.position_for_offset(s).x };
                let mut x1 = layout.position_for_offset(e).x;
                if row_contains_newline_sel {
                    x1 += px(5.);
                }
                if e >= row.end && x1 < x0 {
                    x1 = x0 + px(5.);
                }
                selections.push(fill(Bounds::from_corners(point(origin.x + x0, origin.y + row_y), point(origin.x + x1, origin.y + row_y + line_height)), theme.text_selection));
            }
        }

        let cursor = if selected_range.is_empty() {
            Some(fill(Bounds::new(point(origin.x + cursor_pos.x, origin.y + cursor_pos.y + px(1.)), size(px(1.5), line_height - px(2.))), theme.accent_hover))
        } else {
            None
        };

        let lines = layout.lines.iter().map(|l| (point(origin.x, origin.y + l.y), l.line.clone())).collect();

        self.input.update(cx, |input, _| {
            input.scroll = scroll;
            input.scroll_to_cursor = false;
        });

        PrepaintState { lines, cursor, selections, layout: if real_layout { Some(layout) } else { Some(InputLayout { lines: Vec::new(), line_height, content_height: line_height, content_width: px(0.) }) } }
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), prepaint: &mut PrepaintState, window: &mut Window, cx: &mut App) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.input.clone()), cx);
        let line_height = window.line_height();
        let mask_bounds = Bounds::new(point(bounds.left() - px(2.), bounds.top()), size(bounds.size.width + px(4.), bounds.size.height));
        window.with_content_mask(Some(ContentMask { bounds: mask_bounds }), |window| {
            for sel in prepaint.selections.drain(..) {
                window.paint_quad(sel);
            }
            for (origin, line) in &prepaint.lines {
                if origin.y + line.size(line_height).height < bounds.top() || origin.y > bounds.bottom() {
                    continue;
                }
                let _ = line.paint(*origin, line_height, TextAlign::Left, None, window, cx);
            }
            if focus_handle.is_focused(window) {
                if let Some(cursor) = prepaint.cursor.take() {
                    window.paint_quad(cursor);
                }
            }
        });
        let layout = prepaint.layout.take();
        self.input.update(cx, |input, _| {
            input.layout = layout;
            input.bounds = Some(bounds);
        });
    }
}
