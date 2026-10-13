//! Parsing and validating terminal drag-and-drop payloads.

use super::*;

/// negative only pastes it.
pub(crate) fn dropped_files(text: &str) -> Option<Vec<PathBuf>> {
    // Read as a shell would first (the usual spellings), then as one raw
    // path per line: a raw path with a space in it falls apart into words,
    // so the first reading refuses it and the second picks it up.
    let as_words = shell_words(text).and_then(|words| {
        words
            .iter()
            .map(|word| word_to_path(word))
            .collect::<Option<Vec<_>>>()
    });
    let as_lines = text
        .lines()
        .map(|line| line.trim_matches(|c: char| c.is_ascii_whitespace()))
        .filter(|line| !line.is_empty())
        .map(word_to_path)
        .collect::<Option<Vec<_>>>();
    [as_words, as_lines]
        .into_iter()
        .flatten()
        .find(|paths| !paths.is_empty() && paths.iter().all(|path| is_existing_file(path)))
}

/// Splits `text` into words the way a POSIX shell would -- on ASCII
/// whitespace only, so a Unicode space inside a name stays inside its word
/// (macOS puts a narrow no-break space before the AM/PM of a screenshot's
/// name, and a terminal does not escape it) -- honouring backslash escapes
/// and single and double quotes, which is how terminals spell a dropped
/// path. `None` for text a shell would still be waiting on: an
/// unterminated quote or a trailing backslash.
pub(super) fn shell_words(text: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    // Set once a word has begun, so an explicitly empty one ('') still counts.
    let mut in_word = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_ascii_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\\' => {
                word.push(chars.next()?);
                in_word = true;
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        // Inside double quotes a backslash only escapes these
                        // four; before anything else it is kept, which is what
                        // leaves a Windows path like "C:\a b\c.txt" intact.
                        '\\' => match chars.next()? {
                            escaped @ ('\\' | '"' | '$' | '`') => word.push(escaped),
                            other => {
                                word.push('\\');
                                word.push(other);
                            }
                        },
                        c => word.push(c),
                    }
                }
            }
            c => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

/// One word of a paste as the absolute path it names: a `file://` URI is
/// decoded (only one for this machine, with an empty or `localhost` host);
/// anything else is taken as it stands. `None` if the result is not
/// absolute, so a relative word is never mistaken for a file.
pub(super) fn word_to_path(word: &str) -> Option<PathBuf> {
    let path = match word.strip_prefix("file://") {
        Some(uri) => percent_decode(uri.strip_prefix("localhost").unwrap_or(uri))?,
        None => word.to_string(),
    };
    let path = PathBuf::from(path);
    path.is_absolute().then_some(path)
}

/// Decodes `%XX` escapes (a URI spells a space as `%20`). `None` if an
/// escape is malformed or the result is not UTF-8.
pub(super) fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// Whether `path` is an existing regular file (following symlinks) -- the
/// only kind of thing a drop can be staged from; a directory, or a path
/// that is not there, is not one.
pub(super) fn is_existing_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}
