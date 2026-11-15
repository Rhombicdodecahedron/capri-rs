//! Opening things: which installed app someone means ("ouvre la calculatrice"), which web address
//! they typed, and where a web search goes. Pure matching; the adapter finds and starts things.

use crate::words::normalize;

/// What people call an app in French, mapped to the English name Windows and most apps use.
const ALIASES: &[(&str, &str)] = &[
    ("calculatrice", "calculator"),
    ("calc", "calculator"),
    ("bloc-notes", "notepad"),
    ("bloc notes", "notepad"),
    ("blocnote", "notepad"),
    ("explorateur", "file explorer"),
    ("explorateur de fichiers", "file explorer"),
    ("explorateur windows", "file explorer"),
    ("mes fichiers", "file explorer"),
    ("parametres", "settings"),
    ("reglages", "settings"),
    ("gestionnaire des taches", "task manager"),
    ("gestionnaire de taches", "task manager"),
    ("terminal", "terminal"),
    ("invite de commandes", "terminal"),
    ("horloge", "clock"),
    ("alarmes", "clock"),
    ("camera", "camera"),
    ("appareil photo", "camera"),
    ("paint", "paint"),
    ("vs code", "visual studio code"),
    ("vscode", "visual studio code"),
    ("word", "word"),
    ("excel", "excel"),
    ("navigateur", "browser"),
    ("le navigateur", "browser"),
];

/// Shortcuts that are installed next to an app but are not the app.
const NOT_THE_APP: &[&str] = &[
    "uninstall",
    "desinstall",
    "readme",
    "release notes",
    "documentation",
    "help",
    "aide",
    "website",
    "license",
    "safe mode",
];

/// The name people would use for the app they asked for: aliases resolved, articles dropped.
pub fn app_query(asked: &str) -> String {
    let mut q = normalize(asked);
    for article in ["l'", "le ", "la ", "les ", "the ", "app ", "appli ", "application ", "logiciel "] {
        if let Some(rest) = q.strip_prefix(article) {
            q = rest.trim().to_string();
        }
    }
    ALIASES.iter().find(|(fr, _)| *fr == q).map_or(q, |(_, en)| (*en).to_string())
}

/// Index of the installed app (by display name) that best fits `asked`.
pub fn best_app(asked: &str, names: &[String]) -> Option<usize> {
    let q = app_query(asked);
    if q.is_empty() {
        return None;
    }
    let q_words: Vec<&str> = q.split_whitespace().collect();
    let score = |name: &str| -> Option<u32> {
        let n = normalize(name);
        if NOT_THE_APP.iter().any(|w| n.contains(w) && !q.contains(w)) {
            return None;
        }
        let words: Vec<&str> = n.split_whitespace().collect();
        if n == q {
            Some(100)
        } else if words.first() == q_words.first() && n.starts_with(&q) {
            Some(80)
        } else if words.windows(q_words.len()).any(|w| w == q_words.as_slice()) {
            Some(60)
        } else if q_words.iter().all(|w| words.iter().any(|x| x.starts_with(w))) {
            Some(40)
        } else {
            None
        }
    };
    names
        .iter()
        .enumerate()
        .filter_map(|(i, n)| score(n).map(|s| (i, s, n.chars().count())))
        // best score, then the shortest name ("Steam" before "Steam VR")
        .max_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)))
        .map(|(i, _, _)| i)
}

/// Endings that make "notes.txt" a file name, not a site.
const FILE_EXTENSIONS: &[&str] = &[
    "txt", "exe", "bat", "cmd", "ps1", "lnk", "pdf", "doc", "docx", "xls", "xlsx", "png", "jpg", "zip", "md", "rs",
    "py", "js",
];

/// A web address she may open: http(s) only, a scheme added when it is missing. `None` for
/// anything else (files, `javascript:`, app links), which must not go through "open a website".
pub fn web_url(typed: &str) -> Option<String> {
    let t = typed.trim();
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = t.to_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Some(t.to_string());
    }
    if lower.contains("://") || lower.contains(':') && !lower.contains('.') {
        return None;
    }
    let host = lower.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    let tld_ok = host
        .rsplit('.')
        .next()
        .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(char::is_alphabetic) && !FILE_EXTENSIONS.contains(&tld));
    (host.contains('.') && tld_ok).then(|| format!("https://{t}"))
}

/// Where to search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSite {
    Web,
    YouTube,
    Wikipedia,
    Maps,
    Images,
}

impl SearchSite {
    pub fn parse(s: &str) -> SearchSite {
        match normalize(s).as_str() {
            "youtube" | "yt" => SearchSite::YouTube,
            "wikipedia" | "wiki" => SearchSite::Wikipedia,
            "maps" | "map" | "google maps" | "carte" | "plan" => SearchSite::Maps,
            "images" | "image" | "photos" => SearchSite::Images,
            _ => SearchSite::Web,
        }
    }
}

fn encode(q: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for b in q.trim().bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(char::from(b)),
            b' ' => out.push('+'),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

pub fn search_url(query: &str, site: SearchSite) -> String {
    let q = encode(query);
    match site {
        SearchSite::Web => format!("https://www.google.com/search?q={q}"),
        SearchSite::YouTube => format!("https://www.youtube.com/results?search_query={q}"),
        SearchSite::Wikipedia => format!("https://wikipedia.org/w/index.php?search={q}"),
        SearchSite::Maps => format!("https://www.google.com/maps/search/{q}"),
        SearchSite::Images => format!("https://www.google.com/search?tbm=isch&q={q}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn finds_the_app_people_mean() {
        let apps =
            names(&["Steam", "Uninstall Steam", "SteamVR", "Discord", "Calculator", "Visual Studio Code", "Notepad"]);
        assert_eq!(best_app("steam", &apps), Some(0));
        assert_eq!(best_app("la calculatrice", &apps), Some(4));
        assert_eq!(best_app("bloc-notes", &apps), Some(6));
        assert_eq!(best_app("vs code", &apps), Some(5));
        assert_eq!(best_app("visual studio", &apps), Some(5));
        assert_eq!(best_app("photoshop", &apps), None);
        assert_eq!(best_app("uninstall steam", &apps), Some(1));
    }

    #[test]
    fn only_web_addresses_count_as_websites() {
        assert_eq!(web_url("youtube.com").as_deref(), Some("https://youtube.com"));
        assert_eq!(web_url("https://github.com/x").as_deref(), Some("https://github.com/x"));
        assert_eq!(web_url("javascript:alert(1)"), None);
        assert_eq!(web_url("file:///C:/Windows"), None);
        assert_eq!(web_url("C:\\Windows\\system32"), None);
        assert_eq!(web_url("hello world"), None);
        assert_eq!(web_url("notes.txt"), None, "a file name, not a site");
        assert_eq!(web_url("rts.ch/info").as_deref(), Some("https://rts.ch/info"));
        assert_eq!(web_url("version1.2"), None);
    }

    #[test]
    fn searches_go_to_the_right_site() {
        assert_eq!(search_url("rust async", SearchSite::Web), "https://www.google.com/search?q=rust+async");
        assert_eq!(SearchSite::parse("YouTube"), SearchSite::YouTube);
        assert!(search_url("café", SearchSite::Maps).ends_with("caf%C3%A9"));
    }
}
