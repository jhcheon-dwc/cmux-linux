//! Opening links from the terminal (Ghostty's OPEN_URL action, e.g. ctrl+click).
//!
//! Under WSL the user's browser lives on the Windows side, so we prefer the
//! WSL helpers (`wslview`, then `explorer.exe`) and fall back to `xdg-open`.
//! The URL is always passed as a single argv entry — never through a shell —
//! and only a small allow-list of schemes is accepted, because Windows openers
//! will happily launch arbitrary protocol handlers (`ms-msdt:` and friends).

/// Schemes the Windows-side openers may receive. `file:` is deliberately not
/// here: a terminal link like `file:///home/...` is a Linux path that Windows
/// cannot resolve, so it goes to `xdg-open` instead.
const WINDOWS_SCHEMES: &[&str] = &["http", "https", "mailto", "ftp"];

/// Schemes any opener may receive.
const ALLOWED_SCHEMES: &[&str] = &["http", "https", "mailto", "ftp", "file"];

/// Returns the lower-cased scheme of `url` (the part before the first ':'),
/// or None if `url` does not start with a valid RFC 3986 scheme.
fn scheme_of(url: &str) -> Option<String> {
    let (scheme, _) = url.split_once(':')?;
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic()
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    Some(scheme.to_ascii_lowercase())
}

/// Decide how to open `url` given which opener programs exist.
///
/// Returns the argv to spawn (program first, URL as its own argument) or an
/// error describing why the URL was rejected. Pure so it can be unit-tested.
pub fn opener_argv(url: &str, has_tool: impl Fn(&str) -> bool) -> Result<Vec<String>, &'static str> {
    let url = url.trim();
    if url.is_empty() {
        return Err("empty url");
    }
    // Control characters (incl. newlines/NUL) never belong in a link, and a
    // leading '-' would be parsed as an option by the opener.
    if url.chars().any(|c| c.is_control()) {
        return Err("url contains control characters");
    }
    // Valid URLs percent-encode spaces and quotes. Raw ones (possible in an
    // OSC 8 target) could split the Windows command line that WSL interop
    // rebuilds for explorer.exe/wslview into extra arguments.
    if url.chars().any(|c| c.is_whitespace() || c == '"') {
        return Err("url contains whitespace or quotes");
    }
    if url.starts_with('-') {
        return Err("url starts with '-'");
    }

    // Ghostty may hand us a bare absolute path (path links). Only xdg-open,
    // which runs on the Linux side, can make sense of it.
    let scheme = if url.starts_with('/') {
        "file".to_string()
    } else {
        scheme_of(url).ok_or("url has no scheme")?
    };
    if !ALLOWED_SCHEMES.contains(&scheme.as_str()) {
        return Err("url scheme not allowed");
    }

    let windows_ok = WINDOWS_SCHEMES.contains(&scheme.as_str());
    if windows_ok && has_tool("wslview") {
        return Ok(vec!["wslview".into(), url.into()]);
    }
    // explorer.exe treats its argument as a shell verb target; restrict it
    // to plain web links.
    if (scheme == "http" || scheme == "https") && has_tool("explorer.exe") {
        return Ok(vec!["explorer.exe".into(), url.into()]);
    }
    if has_tool("xdg-open") {
        return Ok(vec!["xdg-open".into(), url.into()]);
    }
    Err("no url opener available")
}

/// True if `name` is an executable file in one of the $PATH directories.
fn tool_in_path(name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        std::fs::metadata(dir.join(name))
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
}

/// Open `url` with the best available opener. Does not block: the child is
/// reaped on a helper thread. Returns the argv used, or why nothing ran.
pub fn open_url(url: &str) -> Result<Vec<String>, String> {
    let argv = opener_argv(url, tool_in_path).map_err(|e| e.to_string())?;
    let mut child = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", argv[0]))?;
    // explorer.exe exits non-zero even on success, so the status is ignored;
    // waiting only avoids leaving a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(list: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |t| list.contains(&t)
    }

    #[test]
    fn test_opener_prefers_wslview_then_explorer_then_xdg_open() {
        let all = tools(&["wslview", "explorer.exe", "xdg-open"]);
        assert_eq!(
            opener_argv("https://example.com/a?b=c&d", &all).unwrap(),
            vec!["wslview", "https://example.com/a?b=c&d"]
        );
        let no_wslview = tools(&["explorer.exe", "xdg-open"]);
        assert_eq!(
            opener_argv("https://example.com", &no_wslview).unwrap(),
            vec!["explorer.exe", "https://example.com"]
        );
        let only_xdg = tools(&["xdg-open"]);
        assert_eq!(
            opener_argv("http://example.com", &only_xdg).unwrap(),
            vec!["xdg-open", "http://example.com"]
        );
        assert!(opener_argv("https://example.com", tools(&[])).is_err());
    }

    #[test]
    fn test_explorer_only_gets_web_links() {
        let win = tools(&["explorer.exe", "xdg-open"]);
        // mailto/file must not go to explorer.exe.
        assert_eq!(opener_argv("mailto:a@b.c", &win).unwrap()[0], "xdg-open");
        assert_eq!(opener_argv("file:///home/u/x.txt", &win).unwrap()[0], "xdg-open");
        // Without xdg-open there is nothing safe left.
        assert!(opener_argv("mailto:a@b.c", tools(&["explorer.exe"])).is_err());
        // file: never goes to wslview either (Linux path).
        let wsl = tools(&["wslview", "xdg-open"]);
        assert_eq!(opener_argv("file:///tmp/x", &wsl).unwrap()[0], "xdg-open");
        assert_eq!(opener_argv("/tmp/x.txt", &wsl).unwrap(), vec!["xdg-open", "/tmp/x.txt"]);
    }

    #[test]
    fn test_opener_rejects_unsafe_urls() {
        let all = tools(&["wslview", "explorer.exe", "xdg-open"]);
        for bad in [
            "",
            "   ",
            "-e https://example.com",
            "--help",
            "ms-msdt:/id PCWDiagnostic",
            "javascript:alert(1)",
            "calc.exe",
            "C:\\Windows\\System32\\calc.exe",
            "https://example.com/\nrm -rf /",
            "https://exa\0mple.com",
            "https://x\" /select,\"C:\\Users\\u\\evil.exe",
            "https://x /select,C:\\evil.exe",
            "https://x\u{3000}y",
            "1http://x",
        ] {
            assert!(opener_argv(bad, &all).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn test_opener_keeps_url_as_single_argument() {
        // Shell metacharacters and non-ASCII are passed through untouched as
        // one argv entry (no shell involved).
        let url = "https://example.com/검색?q=a;b|c&$(x)";
        let argv = opener_argv(url, tools(&["xdg-open"])).unwrap();
        assert_eq!(argv, vec!["xdg-open", url]);
        // Scheme matching is case-insensitive.
        assert!(opener_argv("HTTPS://EXAMPLE.COM", tools(&["xdg-open"])).is_ok());
    }
}
