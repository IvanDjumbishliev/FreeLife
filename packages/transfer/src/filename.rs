/// Longest file name we'll create, in bytes. Most filesystems allow 255.
const MAX_LEN: usize = 200;

const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Turn a peer-supplied file name into one that's safe to create in the
/// download directory, or `None` if nothing usable is left.
///
/// Drops any directory components (so `../../.bashrc` becomes `bashrc`),
/// characters that are invalid on Windows, control characters, and leading
/// dots (no hidden files). It also defuses Windows device names like `CON`.
pub fn sanitize_filename(raw: &str) -> Option<String> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        .collect();
    // Windows silently strips trailing dots and spaces; leading dots hide files.
    let mut name = cleaned
        .trim()
        .trim_end_matches(['.', ' '])
        .trim_start_matches('.')
        .trim()
        .to_string();
    if name.is_empty() {
        return None;
    }

    let stem = name.split('.').next().unwrap_or_default();
    if WINDOWS_RESERVED
        .iter()
        .any(|r| r.eq_ignore_ascii_case(stem.trim()))
    {
        name.insert(0, '_');
    }

    if name.len() > MAX_LEN {
        name = truncate_keeping_extension(&name, MAX_LEN);
    }
    Some(name)
}

fn truncate_keeping_extension(name: &str, max: usize) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if name.len() - dot <= 16 => name.split_at(dot),
        _ => (name, ""),
    };
    let mut end = max - ext.len();
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{ext}", &stem[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_ordinary_names() {
        for name in [
            "photo.jpg",
            "Résumé 2026.pdf",
            "archive.tar.gz",
            "日本語.txt",
            "a b c",
        ] {
            assert_eq!(sanitize_filename(name).as_deref(), Some(name));
        }
    }

    #[test]
    fn strips_path_traversal() {
        assert_eq!(
            sanitize_filename("../../etc/passwd").as_deref(),
            Some("passwd")
        );
        assert_eq!(
            sanitize_filename("..\\..\\boot.ini").as_deref(),
            Some("boot.ini")
        );
        assert_eq!(
            sanitize_filename("/abs/path/x.txt").as_deref(),
            Some("x.txt")
        );
        assert_eq!(
            sanitize_filename("C:\\Users\\x\\y.doc").as_deref(),
            Some("y.doc")
        );
        assert_eq!(sanitize_filename("../.bashrc").as_deref(), Some("bashrc"));
    }

    #[test]
    fn rejects_names_with_nothing_left() {
        for name in ["", ".", "..", "...", "  ", "dir/", "\u{0}", "***"] {
            assert_eq!(sanitize_filename(name), None, "{name:?}");
        }
    }

    #[test]
    fn removes_invalid_and_control_characters() {
        assert_eq!(
            sanitize_filename("a<b>c:d\"e|f?g*h\u{7}.txt").as_deref(),
            Some("abcdefgh.txt")
        );
        assert_eq!(
            sanitize_filename("name.txt. . ").as_deref(),
            Some("name.txt")
        );
    }

    #[test]
    fn defuses_windows_device_names() {
        assert_eq!(sanitize_filename("CON").as_deref(), Some("_CON"));
        assert_eq!(sanitize_filename("nul.txt").as_deref(), Some("_nul.txt"));
        assert_eq!(
            sanitize_filename("com1.tar.gz").as_deref(),
            Some("_com1.tar.gz")
        );
        assert_eq!(
            sanitize_filename("console.txt").as_deref(),
            Some("console.txt")
        );
    }

    #[test]
    fn truncates_long_names_keeping_extension() {
        let long = format!("{}.jpeg", "é".repeat(300));
        let out = sanitize_filename(&long).unwrap();
        assert!(out.len() <= MAX_LEN);
        assert!(out.ends_with(".jpeg"));
    }
}
