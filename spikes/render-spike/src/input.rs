use crate::editor_view::EditorView;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};

/// Translates one real winit key event into a real `Document` edit.
/// Returns `true` if the document actually changed (the caller uses this to
/// decide whether a re-shape/re-render is needed).
pub fn handle_key_event(editor: &mut EditorView, event: &KeyEvent) -> bool {
    if event.state != ElementState::Pressed {
        return false;
    }

    match &event.logical_key {
        Key::Named(NamedKey::Backspace) => {
            editor.backspace();
            true
        }
        Key::Named(NamedKey::Enter) => {
            editor.insert_at_cursor("\n");
            true
        }
        Key::Named(NamedKey::Space) => {
            editor.insert_at_cursor(" ");
            true
        }
        _ => {
            // `text` carries the actual characters this keypress produces,
            // already accounting for the OS keyboard layout -- e.g. it's
            // `None` for pure modifier/navigation keys, exactly the filter
            // this spike wants rather than hand-rolling a keycode table.
            if let Some(text) = &event.text {
                if !text.is_empty() && text.chars().all(|c| !c.is_control()) {
                    editor.insert_at_cursor(text);
                    return true;
                }
            }
            false
        }
    }
}
