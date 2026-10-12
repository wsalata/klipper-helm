//! Single-line text editing for the on-screen and physical keyboards.
//! The text is kept split at the cursor, so the UI can draw the cursor
//! between the two parts without any string slicing in Slint.

/// Applies one key to the text around the cursor.
///
/// Named keys: "BS" (delete before the cursor), "DEL" (delete after it),
/// "LEFT", "RIGHT", "HOME", "END" and "CLR" (clear everything).
/// Anything else is inserted at the cursor, unless it contains control
/// characters or keys Slint reports as private-use characters (arrows,
/// function keys, ...), which would otherwise show up as odd glyphs.
pub fn apply(before: &mut String, after: &mut String, key: &str) {
    match key {
        "BS" => {
            before.pop();
        }
        "DEL" => {
            if let Some(c) = after.chars().next() {
                after.drain(..c.len_utf8());
            }
        }
        "LEFT" => {
            if let Some(c) = before.pop() {
                after.insert(0, c);
            }
        }
        "RIGHT" => {
            if let Some(c) = after.chars().next() {
                after.drain(..c.len_utf8());
                before.push(c);
            }
        }
        "HOME" => {
            after.insert_str(0, before);
            before.clear();
        }
        "END" => {
            before.push_str(after);
            after.clear();
        }
        "CLR" => {
            before.clear();
            after.clear();
        }
        text if !text.is_empty() && text.chars().all(is_printable) => before.push_str(text),
        _ => {}
    }
}

fn is_printable(c: char) -> bool {
    // Slint maps special keys into the Unicode private use area (e.g. U+F702 = left arrow)
    !c.is_control() && !('\u{E000}'..='\u{F8FF}').contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(keys: &[&str]) -> (String, String) {
        let (mut before, mut after) = (String::new(), String::new());
        for key in keys {
            apply(&mut before, &mut after, key);
        }
        (before, after)
    }

    #[test]
    fn typing_and_cursor_movement() {
        assert_eq!(run(&["a", "b", "c", "LEFT", "LEFT", "X"]), ("aX".into(), "bc".into()));
        assert_eq!(run(&["a", "b", "HOME", "X", "END", "Y"]), ("XabY".into(), "".into()));
        assert_eq!(run(&["a", "b", "LEFT", "RIGHT", "RIGHT"]), ("ab".into(), "".into()));
    }

    #[test]
    fn deleting() {
        assert_eq!(run(&["a", "b", "c", "LEFT", "BS"]), ("a".into(), "c".into()));
        assert_eq!(run(&["a", "b", "c", "HOME", "DEL"]), ("".into(), "bc".into()));
        assert_eq!(run(&["a", "LEFT", "BS", "HOME", "LEFT"]), ("".into(), "a".into()));
        assert_eq!(run(&["a", "b", "LEFT", "CLR"]), ("".into(), "".into()));
    }

    #[test]
    fn multibyte_characters() {
        assert_eq!(run(&["ż", "ó", "LEFT", "DEL", "x"]), ("żx".into(), "".into()));
    }

    #[test]
    fn special_keys_are_not_inserted() {
        // Up arrow, F1 and Escape as Slint reports them
        assert_eq!(run(&["a", "\u{F700}", "\u{F704}", "\u{1b}"]), ("a".into(), "".into()));
    }
}
