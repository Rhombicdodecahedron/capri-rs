//! Driving other apps the way Playwright drives a web page, as plain data: a window's accessibility
//! tree flattened into [`Element`]s, [`Locator`]s that point at a control by role and name (and are
//! looked up again on every action, so they survive re-renders), key chords, and the rules that keep
//! her hands off private windows and make her ask before a risky button.

use serde::{Deserialize, Serialize};

use crate::Risk;
use crate::geometry::Rect;
use crate::words::normalize;

/// What kind of control an element is, from the accessibility tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Button,
    CheckBox,
    ComboBox,
    Edit,
    Link,
    List,
    ListItem,
    Menu,
    MenuItem,
    RadioButton,
    Slider,
    Tab,
    TabItem,
    Text,
    Tree,
    TreeItem,
    Table,
    Row,
    Document,
    Group,
    Image,
    Window,
    Other,
}

impl Role {
    pub const ALL: [Role; 23] = [
        Role::Button,
        Role::CheckBox,
        Role::ComboBox,
        Role::Edit,
        Role::Link,
        Role::List,
        Role::ListItem,
        Role::Menu,
        Role::MenuItem,
        Role::RadioButton,
        Role::Slider,
        Role::Tab,
        Role::TabItem,
        Role::Text,
        Role::Tree,
        Role::TreeItem,
        Role::Table,
        Role::Row,
        Role::Document,
        Role::Group,
        Role::Image,
        Role::Window,
        Role::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Button => "button",
            Role::CheckBox => "checkbox",
            Role::ComboBox => "combobox",
            Role::Edit => "textbox",
            Role::Link => "link",
            Role::List => "list",
            Role::ListItem => "listitem",
            Role::Menu => "menu",
            Role::MenuItem => "menuitem",
            Role::RadioButton => "radio",
            Role::Slider => "slider",
            Role::Tab => "tablist",
            Role::TabItem => "tab",
            Role::Text => "text",
            Role::Tree => "tree",
            Role::TreeItem => "treeitem",
            Role::Table => "table",
            Role::Row => "row",
            Role::Document => "document",
            Role::Group => "group",
            Role::Image => "image",
            Role::Window => "window",
            Role::Other => "other",
        }
    }

    /// Accepts the ARIA names the snapshot prints, plus a few synonyms models like to use.
    pub fn parse(s: &str) -> Option<Role> {
        let s = normalize(s);
        let alias = match s.as_str() {
            "edit" | "input" | "textfield" | "text field" | "searchbox" | "search" => Some(Role::Edit),
            "check box" => Some(Role::CheckBox),
            "combo box" | "dropdown" | "select" => Some(Role::ComboBox),
            "hyperlink" => Some(Role::Link),
            "list item" | "item" => Some(Role::ListItem),
            "menu item" => Some(Role::MenuItem),
            "radio button" | "radiobutton" => Some(Role::RadioButton),
            "label" => Some(Role::Text),
            _ => None,
        };
        alias.or_else(|| Role::ALL.into_iter().find(|r| r.as_str() == s))
    }

    /// Controls worth listing for someone who wants to act on the window.
    pub fn interactive(self) -> bool {
        matches!(
            self,
            Role::Button
                | Role::CheckBox
                | Role::ComboBox
                | Role::Edit
                | Role::Link
                | Role::ListItem
                | Role::MenuItem
                | Role::RadioButton
                | Role::Slider
                | Role::TabItem
                | Role::TreeItem
                | Role::Row
        )
    }
}

/// A top-level window on the desktop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    /// The OS handle, stable while the window lives.
    pub id: u64,
    pub title: String,
    /// The program's file name, e.g. `Spotify.exe`.
    pub process: String,
    pub pid: u32,
}

impl WindowInfo {
    /// `Spotify.exe` -> `spotify`.
    pub fn app(&self) -> String {
        let p = normalize(&self.process);
        p.strip_suffix(".exe").map_or(p.clone(), str::to_string)
    }
}

/// Which window to act on.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WindowQuery {
    /// Exactly this window (from a previous lookup).
    pub id: Option<u64>,
    /// The program, with or without `.exe`, any case.
    pub process: Option<String>,
    /// Part of the title, any case or accents.
    pub title: Option<String>,
}

impl WindowQuery {
    pub fn id(id: u64) -> Self {
        Self { id: Some(id), ..Self::default() }
    }

    pub fn process(p: impl Into<String>) -> Self {
        Self { process: Some(p.into()), ..Self::default() }
    }

    pub fn matches(&self, w: &WindowInfo) -> bool {
        if self.id.is_some_and(|id| id != w.id) {
            return false;
        }
        if let Some(p) = &self.process {
            let p = normalize(p);
            if p.strip_suffix(".exe").unwrap_or(&p) != w.app() {
                return false;
            }
        }
        if let Some(t) = &self.title {
            if !normalize(&w.title).contains(&normalize(t)) {
                return false;
            }
        }
        true
    }

    /// The window someone named in plain words ("spotify", "Discord", "Bloc-notes"): the program
    /// whose name it is, else the first title containing it. Windows without a title are skipped.
    pub fn pick<'a>(name: &str, windows: &'a [WindowInfo]) -> Option<&'a WindowInfo> {
        let n = normalize(name);
        let n = n.strip_suffix(".exe").unwrap_or(&n).to_string();
        if n.is_empty() {
            return None;
        }
        let titled = || windows.iter().filter(|w| !w.title.trim().is_empty());
        titled()
            .find(|w| w.app() == n)
            .or_else(|| titled().find(|w| normalize(&w.title) == n))
            .or_else(|| titled().find(|w| normalize(&w.title).contains(&n)))
            .or_else(|| titled().find(|w| w.app().contains(&n)))
    }
}

/// One control in a window, as the accessibility tree describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub role: Role,
    pub name: String,
    /// What a text box, slider or combo box holds.
    pub value: Option<String>,
    pub enabled: bool,
    /// Checked / pressed state of a toggle, when it has one.
    pub checked: Option<bool>,
    /// Where it is on screen (physical pixels), when the app says.
    pub bounds: Option<Rect>,
}

impl Element {
    pub fn new(role: Role, name: impl Into<String>) -> Self {
        Self { role, name: name.into(), value: None, enabled: true, checked: None, bounds: None }
    }

    #[must_use]
    pub fn at(mut self, bounds: Rect) -> Self {
        self.bounds = Some(bounds);
        self
    }

    #[must_use]
    pub fn valued(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameMatch {
    #[default]
    Exact,
    Prefix,
    Contains,
}

/// When several controls match, which one to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prefer {
    /// The `nth` in tree order.
    #[default]
    Order,
    /// The lowest on screen (a player bar sits at the bottom of the window).
    Lowest,
}

/// Points at a control the way Playwright's `getByRole(role, { name })` does. Names are compared
/// without case or accents, and any of `names` will do, so one locator covers every UI language.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Locator {
    pub role: Option<Role>,
    pub names: Vec<String>,
    pub matching: NameMatch,
    /// Names that must not match even if `names` does (the "Play" transport button is not a result).
    pub except: Vec<String>,
    pub nth: usize,
    pub prefer: Prefer,
    /// Skip greyed-out controls.
    pub enabled_only: bool,
}

impl Locator {
    pub fn role(role: Role) -> Self {
        Self { role: Some(role), enabled_only: true, ..Self::default() }
    }

    /// Any control with this name.
    pub fn named(name: impl Into<String>) -> Self {
        Self { names: vec![name.into()], enabled_only: true, ..Self::default() }
    }

    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.names.push(name.into());
        self
    }

    #[must_use]
    pub fn any_of(mut self, names: &[&str]) -> Self {
        self.names.extend(names.iter().map(|n| (*n).to_string()));
        self
    }

    #[must_use]
    pub fn prefix(mut self) -> Self {
        self.matching = NameMatch::Prefix;
        self
    }

    #[must_use]
    pub fn contains(mut self) -> Self {
        self.matching = NameMatch::Contains;
        self
    }

    #[must_use]
    pub fn except(mut self, names: &[&str]) -> Self {
        self.except.extend(names.iter().map(|n| (*n).to_string()));
        self
    }

    #[must_use]
    pub fn nth(mut self, n: usize) -> Self {
        self.nth = n;
        self
    }

    #[must_use]
    pub fn lowest(mut self) -> Self {
        self.prefer = Prefer::Lowest;
        self
    }

    pub fn matches(&self, e: &Element) -> bool {
        if self.role.is_some_and(|r| r != e.role) || (self.enabled_only && !e.enabled) {
            return false;
        }
        let name = normalize(&e.name);
        if self.except.iter().any(|x| normalize(x) == name) {
            return false;
        }
        if self.names.is_empty() {
            return true;
        }
        self.names.iter().map(|n| normalize(n)).any(|n| match self.matching {
            NameMatch::Exact => name == n,
            NameMatch::Prefix => name.starts_with(&n),
            NameMatch::Contains => name.contains(&n),
        })
    }

    /// Index of the element this locator points at.
    pub fn pick(&self, elements: &[Element]) -> Option<usize> {
        let hits = elements.iter().enumerate().filter(|(_, e)| self.matches(e));
        match self.prefer {
            Prefer::Order => hits.map(|(i, _)| i).nth(self.nth),
            Prefer::Lowest => hits.max_by(|(_, a), (_, b)| bottom(a).total_cmp(&bottom(b))).map(|(i, _)| i),
        }
    }

    /// Short human description, for logs and approval questions: `button "Next"`.
    pub fn describe(&self) -> String {
        let role = self.role.map_or("control", Role::as_str);
        match self.names.first() {
            Some(n) => format!("{role} \"{n}\""),
            None => role.to_string(),
        }
    }
}

fn bottom(e: &Element) -> f32 {
    e.bounds.map_or(f32::MIN, |b| b.bottom())
}

/// A window and its controls at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub window: WindowInfo,
    pub elements: Vec<Element>,
}

/// `e12` -> index 11, as printed by [`Snapshot::render`].
pub fn parse_ref(r: &str) -> Option<usize> {
    r.trim().trim_start_matches('[').trim_end_matches(']').strip_prefix('e')?.parse::<usize>().ok()?.checked_sub(1)
}

impl Snapshot {
    /// The page as a model reads it, one control per line with a ref to act on, like Playwright's
    /// ARIA snapshot: `- e3 button "Next"`. Only controls with a name or a value are listed, at most
    /// `max` of them.
    pub fn render(&self, max: usize) -> String {
        use std::fmt::Write as _;
        let mut out = format!("window \"{}\" ({})\n", clip(&self.window.title, 120), self.window.process);
        let mut shown = 0;
        for (i, e) in self.elements.iter().enumerate() {
            if e.name.trim().is_empty() && e.value.as_deref().is_none_or(|v| v.trim().is_empty()) {
                continue;
            }
            if shown == max {
                out.push_str("- … more controls not shown\n");
                break;
            }
            shown += 1;
            let _ = write!(out, "- e{} {} \"{}\"", i + 1, e.role.as_str(), clip(&e.name, 90));
            if let Some(v) = e.value.as_deref().filter(|v| !v.trim().is_empty()) {
                let _ = write!(out, " = \"{}\"", clip(v, 60));
            }
            match e.checked {
                Some(true) => out.push_str(" [checked]"),
                Some(false) => out.push_str(" [unchecked]"),
                None => {}
            }
            if !e.enabled {
                out.push_str(" [disabled]");
            }
            out.push('\n');
        }
        out
    }

    /// A locator that finds element `index` again later: same role and name, same rank among the
    /// controls that share them.
    pub fn locator_for(&self, index: usize) -> Option<Locator> {
        let e = self.elements.get(index)?;
        let nth = self.elements[..index].iter().filter(|x| x.role == e.role && x.name == e.name).count();
        let mut l = Locator::role(e.role).name(e.name.clone()).nth(nth);
        l.enabled_only = false;
        Some(l)
    }
}

fn clip(s: &str, n: usize) -> String {
    let s = s.replace(['\n', '\r', '"'], " ");
    if s.chars().count() <= n { s } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

// -- keys --------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Win,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Escape,
    Space,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

/// A key with modifiers held, e.g. `ctrl+right`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyChord {
    pub modifiers: Vec<Modifier>,
    pub key: Key,
}

impl KeyChord {
    pub fn new(modifiers: &[Modifier], key: Key) -> Self {
        Self { modifiers: modifiers.to_vec(), key }
    }

    /// `"ctrl+shift+s"`, `"Enter"`, `"alt+F4"`, `"ctrl + →"`.
    pub fn parse(s: &str) -> Option<KeyChord> {
        let parts: Vec<String> = s.split('+').map(|p| normalize(p).replace(' ', "")).collect();
        let (last, mods) = parts.split_last()?;
        let mut modifiers = Vec::new();
        for m in mods {
            let m = match m.as_str() {
                "ctrl" | "control" | "ctl" | "strg" => Modifier::Ctrl,
                "alt" | "option" | "opt" => Modifier::Alt,
                "shift" | "maj" => Modifier::Shift,
                "win" | "windows" | "meta" | "super" | "cmd" | "command" => Modifier::Win,
                _ => return None,
            };
            if !modifiers.contains(&m) {
                modifiers.push(m);
            }
        }
        let key = match last.as_str() {
            "enter" | "return" | "entree" => Key::Enter,
            "tab" => Key::Tab,
            "esc" | "escape" | "echap" => Key::Escape,
            "space" | "espace" | "spacebar" => Key::Space,
            "backspace" => Key::Backspace,
            "delete" | "del" | "suppr" => Key::Delete,
            "up" | "arrowup" | "haut" | "↑" => Key::Up,
            "down" | "arrowdown" | "bas" | "↓" => Key::Down,
            "left" | "arrowleft" | "gauche" | "←" => Key::Left,
            "right" | "arrowright" | "droite" | "→" => Key::Right,
            "home" => Key::Home,
            "end" | "fin" => Key::End,
            "pageup" | "pgup" => Key::PageUp,
            "pagedown" | "pgdn" => Key::PageDown,
            f if f.len() > 1 && f.starts_with('f') && f[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => {
                Key::F(f[1..].parse().ok()?)
            }
            c if c.chars().count() == 1 => Key::Char(c.chars().next()?),
            _ => return None,
        };
        Some(KeyChord { modifiers, key })
    }

    fn has(&self, m: Modifier) -> bool {
        self.modifiers.contains(&m)
    }

    /// Chords that close, delete, send or reach the system: always confirmed.
    pub fn risk(&self) -> Risk {
        let (ctrl, alt, win) = (self.has(Modifier::Ctrl), self.has(Modifier::Alt), self.has(Modifier::Win));
        let destructive = match self.key {
            // closing windows, deleting, sending (Enter sends in every chat app)
            Key::F(4) if alt || ctrl => true,
            Key::Delete | Key::Enter => true,
            Key::Char('w' | 'q') if ctrl => true,
            // Win+R runs anything, Win+X is the power menu, Win+L locks: only desktop and arrows are fine
            _ if win => !matches!(self.key, Key::Char('d' | 'e') | Key::Up | Key::Down | Key::Left | Key::Right),
            _ => false,
        };
        if destructive { Risk::Destructive } else { Risk::Reversible }
    }
}

impl std::fmt::Display for KeyChord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for m in &self.modifiers {
            let name = match m {
                Modifier::Ctrl => "ctrl",
                Modifier::Alt => "alt",
                Modifier::Shift => "shift",
                Modifier::Win => "win",
            };
            write!(f, "{name}+")?;
        }
        match self.key {
            Key::Char(c) => write!(f, "{c}"),
            Key::F(n) => write!(f, "f{n}"),
            k => write!(f, "{}", format!("{k:?}").to_lowercase()),
        }
    }
}

// -- what she may touch ----------------------------------------------------------------------

/// Stems of labels on buttons that delete, send, pay or sign out: pressing one is never something
/// the model decides alone.
const RISKY: &[&str] = &[
    "delete",
    "remove",
    "erase",
    "trash",
    "discard",
    "uninstall",
    "format",
    "reset",
    "wipe",
    "overwrite",
    "replace",
    "send",
    "submit",
    "publish",
    "post",
    "reply all",
    "forward",
    "share",
    "pay",
    "buy",
    "purchase",
    "order",
    "checkout",
    "subscribe",
    "unsubscribe",
    "transfer",
    "sign out",
    "log out",
    "logout",
    "close account",
    "shut down",
    "shutdown",
    "restart",
    "end task",
    "supprim",
    "efface",
    "corbeille",
    "desinstall",
    "reinitialis",
    "envoy",
    "publi",
    "partag",
    "transf",
    "payer",
    "achet",
    "command",
    "abonn",
    "virement",
    "deconnex",
    "se deconnecter",
    "eteindre",
    "redemarrer",
    "fin de tache",
    "ecraser",
    "remplacer",
    "vider",
    "loschen",
    "senden",
    "kaufen",
    "bezahlen",
    "abmelden",
    "eliminar",
    "borrar",
    "enviar",
    "comprar",
    "pagar",
    "elimina",
    "invia",
    "acquista",
    "paga",
];

/// How risky pressing a control with this name is.
pub fn risk_of(name: &str) -> Risk {
    let n = normalize(name);
    if RISKY.iter().any(|w| n.contains(w)) { Risk::Destructive } else { Risk::Reversible }
}

/// Typing into a field and pressing Enter: a search is harmless, anything else may send a message.
pub fn submit_risk(field: &Element) -> Risk {
    const SEARCHY: &[&str] = &[
        "search",
        "recherche",
        "cherche",
        "find",
        "trouver",
        "query",
        "filter",
        "filtre",
        "address",
        "adresse",
        "url",
        "suche",
        "buscar",
        "cerca",
        "what do you want to play",
        "que souhaitez-vous ecouter",
        "go to",
    ];
    let n = normalize(&field.name);
    if field.role == Role::Edit && SEARCHY.iter().any(|w| n.contains(w)) { Risk::Reversible } else { Risk::Destructive }
}

/// Windows she must never read or touch: password managers, banking and payments, private browsing,
/// sign-in and credential prompts, plus the user's own list.
pub fn private(window: &WindowInfo, never_touch: &[String]) -> bool {
    const PRIVATE: &[&str] = &[
        "1password",
        "bitwarden",
        "keepass",
        "lastpass",
        "dashlane",
        "proton pass",
        "enpass",
        "password",
        "mot de passe",
        "passwort",
        "contrasena",
        "credential",
        "windows security",
        "securite windows",
        "inprivate",
        "incognito",
        "navigation privee",
        "private browsing",
        "fenetre privee",
        "e-banking",
        "ebanking",
        "banking",
        "banque",
        "bank",
        "postfinance",
        "twint",
        "paypal",
        "revolut",
        "raiffeisen",
        "credit suisse",
        "ubs ",
        "sign in",
        "log in",
        "login",
        "connexion",
        "se connecter",
        "authenticator",
        "2fa",
        "verification code",
        "code de verification",
    ];
    let hay = format!("{} {}", normalize(&window.title), window.app());
    PRIVATE.iter().any(|w| hay.contains(w))
        || never_touch.iter().map(|w| normalize(w)).any(|w| !w.is_empty() && hay.contains(&w))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(title: &str, process: &str) -> WindowInfo {
        WindowInfo { id: 1, title: title.into(), process: process.into(), pid: 7 }
    }

    #[test]
    fn locators_ignore_case_accents_and_pick_by_language() {
        let els = [Element::new(Role::Button, "Précédent"), Element::new(Role::Button, "Suivant")];
        let next = Locator::role(Role::Button).any_of(&["Next", "suivant"]);
        assert_eq!(next.pick(&els), Some(1));
        assert_eq!(Locator::role(Role::Button).name("precedent").pick(&els), Some(0));
        assert_eq!(Locator::role(Role::Edit).name("Suivant").pick(&els), None);
    }

    #[test]
    fn lowest_prefers_the_player_bar() {
        let els = [
            Element::new(Role::Button, "Play").at(Rect::new(0.0, 100.0, 50.0, 50.0)),
            Element::new(Role::Button, "Play").at(Rect::new(0.0, 900.0, 30.0, 30.0)),
        ];
        assert_eq!(Locator::role(Role::Button).name("play").lowest().pick(&els), Some(1));
        assert_eq!(Locator::role(Role::Button).name("play").pick(&els), Some(0));
    }

    #[test]
    fn prefix_with_exceptions_finds_results_not_the_transport() {
        let els = [Element::new(Role::Button, "Play"), Element::new(Role::Button, "Play Get Lucky by Daft Punk")];
        let l = Locator::role(Role::Button).name("play").prefix().except(&["play"]);
        assert_eq!(l.pick(&els), Some(1));
    }

    #[test]
    fn snapshot_refs_round_trip_to_locators() {
        let snap = Snapshot {
            window: win("Spotify Premium", "Spotify.exe"),
            elements: vec![
                Element::new(Role::Button, "Like"),
                Element::new(Role::Group, ""),
                Element::new(Role::Button, "Like"),
                Element::new(Role::Edit, "Search").valued("daft"),
            ],
        };
        let text = snap.render(50);
        assert!(text.contains("- e3 button \"Like\""), "{text}");
        assert!(text.contains("- e4 textbox \"Search\" = \"daft\""), "{text}");
        assert!(!text.contains("e2 "), "nameless groups are skipped: {text}");
        let i = parse_ref("e3").unwrap();
        assert_eq!(snap.locator_for(i).unwrap().pick(&snap.elements), Some(2));
        assert_eq!(parse_ref("[e1]"), Some(0));
        assert_eq!(parse_ref("e0"), None);
    }

    #[test]
    fn windows_are_found_by_app_or_title() {
        let ws =
            [win("", "Spotify.exe"), win("Daft Punk - Get Lucky", "Spotify.exe"), win("Bloc-notes", "notepad.exe")];
        assert_eq!(WindowQuery::pick("Spotify", &ws).map(|w| w.title.as_str()), Some("Daft Punk - Get Lucky"));
        assert_eq!(WindowQuery::pick("bloc-notes", &ws).map(|w| w.process.as_str()), Some("notepad.exe"));
        assert!(WindowQuery::process("spotify.EXE").matches(&ws[0]));
        assert!(WindowQuery::pick("word", &ws).is_none());
    }

    #[test]
    fn key_chords_parse_and_carry_risk() {
        let c = KeyChord::parse("Ctrl + Right").unwrap();
        assert_eq!(c, KeyChord::new(&[Modifier::Ctrl], Key::Right));
        assert_eq!(c.to_string(), "ctrl+right");
        assert_eq!(KeyChord::parse("alt+f4").unwrap().risk(), Risk::Destructive);
        assert_eq!(KeyChord::parse("enter").unwrap().risk(), Risk::Destructive);
        assert_eq!(KeyChord::parse("space").unwrap().risk(), Risk::Reversible);
        assert_eq!(KeyChord::parse("ctrl+s").unwrap().risk(), Risk::Reversible);
        assert_eq!(KeyChord::parse("ctrl+w").unwrap().risk(), Risk::Destructive);
        assert!(KeyChord::parse("hyper+x").is_none());
        assert!(KeyChord::parse("").is_none());
    }

    #[test]
    fn risky_labels_in_several_languages() {
        for name in ["Delete", "Supprimer le fichier", "Envoyer", "Send message", "Payer maintenant", "Löschen"] {
            assert_eq!(risk_of(name), Risk::Destructive, "{name}");
        }
        for name in ["Next", "Suivant", "Play", "Enable shuffle", "Ajouter aux Titres likés"] {
            assert_eq!(risk_of(name), Risk::Reversible, "{name}");
        }
        assert_eq!(submit_risk(&Element::new(Role::Edit, "Rechercher")), Risk::Reversible);
        assert_eq!(submit_risk(&Element::new(Role::Edit, "Message #général")), Risk::Destructive);
    }

    #[test]
    fn private_windows_are_off_limits() {
        assert!(private(&win("Bitwarden", "Bitwarden.exe"), &[]));
        assert!(private(&win("PostFinance E-Finance - Google Chrome", "chrome.exe"), &[]));
        assert!(private(&win("Nouvel onglet - Navigation privée - Mozilla Firefox", "firefox.exe"), &[]));
        assert!(private(&win("Journal", "obsidian.exe"), &["journal".into()]));
        assert!(!private(&win("Spotify Premium", "Spotify.exe"), &[]));
    }
}
