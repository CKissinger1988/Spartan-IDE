use crate::editor_view::{EditEffect, EditorView};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};

/// Translates one real winit key event into a real `Document` edit. Returns
/// the resulting `EditEffect` so the caller knows whether a cheap per-line
/// reshape suffices or a full-document reshape is needed (see
/// `EditorView::insert_at_cursor`/`backspace`'s doc comments).
pub fn handle_key_event(editor: &mut EditorView, event: &KeyEvent) -> EditEffect {
    if event.state != ElementState::Pressed {
        return EditEffect::None;
    }

    match &event.logical_key {
        Key::Named(NamedKey::Backspace) => editor.backspace(),
        Key::Named(NamedKey::Enter) => editor.insert_at_cursor("\n"),
        Key::Named(NamedKey::Space) => editor.insert_at_cursor(" "),
        _ => {
            // `text` carries the actual characters this keypress produces,
            // already accounting for the OS keyboard layout -- e.g. it's
            // `None` for pure modifier/navigation keys, exactly the filter
            // this spike wants rather than hand-rolling a keycode table.
            if let Some(text) = &event.text {
                if !text.is_empty() && text.chars().all(|c| !c.is_control()) {
                    return editor.insert_at_cursor(text);
                }
            }
            EditEffect::None
        }
    }
}
