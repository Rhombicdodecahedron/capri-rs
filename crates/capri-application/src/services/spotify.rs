//! Use case: Spotify driven through its own window, like a person (or Playwright) would: find the
//! button by its name and press it, search and click the result. No Web API and no account keys.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use capri_domain::Risk;
use capri_domain::spotify::{self, SpotifyAction};
use capri_domain::ui::{WindowInfo, WindowQuery};

use crate::ports::{Opener, Tool, ToolError, ToolSpec, UiAction, UiDriver, UiError};

/// Name of the tool in the [`ToolRunner`](crate::services::ToolRunner).
pub const SPOTIFY_TOOL: &str = "spotify";

/// How long to wait for things that take time on a real PC.
#[derive(Debug, Clone, Copy)]
pub struct Patience {
    /// Spotify starting cold.
    pub launch: Duration,
    /// Search results showing up.
    pub results: Duration,
    pub poll: Duration,
}

impl Default for Patience {
    fn default() -> Self {
        Self { launch: Duration::from_secs(15), results: Duration::from_secs(8), poll: Duration::from_millis(300) }
    }
}

pub struct SpotifyRemote {
    ui: Arc<dyn UiDriver>,
    opener: Arc<dyn Opener>,
    patience: Patience,
}

impl std::fmt::Debug for SpotifyRemote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpotifyRemote").field("patience", &self.patience).finish_non_exhaustive()
    }
}

fn failed(e: &UiError) -> ToolError {
    match e {
        UiError::Unsupported => ToolError::Unavailable(e.to_string()),
        _ => ToolError::Failed(e.to_string()),
    }
}

impl SpotifyRemote {
    pub fn new(ui: Arc<dyn UiDriver>, opener: Arc<dyn Opener>) -> Self {
        Self { ui, opener, patience: Patience::default() }
    }

    #[must_use]
    pub fn with_patience(mut self, patience: Patience) -> Self {
        self.patience = patience;
        self
    }

    /// Spotify's main window, if it is open (it has a title; its helper windows don't).
    async fn window(&self) -> Result<Option<WindowInfo>, ToolError> {
        let windows = self.ui.windows().await.map_err(|e| failed(&e))?;
        let mut mine: Vec<WindowInfo> = windows.into_iter().filter(spotify::is_spotify).collect();
        mine.sort_by_key(|w| w.title.trim().is_empty());
        Ok(mine.into_iter().next())
    }

    async fn wait_for_window(&self) -> Result<WindowInfo, ToolError> {
        let deadline = tokio::time::Instant::now() + self.patience.launch;
        loop {
            if let Some(w) = self.window().await? {
                return Ok(w);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ToolError::Failed("Spotify did not open (is it installed?)".into()));
            }
            tokio::time::sleep(self.patience.poll).await;
        }
    }

    async fn open(&self) -> Result<WindowInfo, ToolError> {
        if let Some(w) = self.window().await? {
            return Ok(w);
        }
        self.opener.open("spotify:").map_err(ToolError::Failed)?;
        self.wait_for_window().await
    }

    /// A player button: pressed through the accessibility tree, or with Spotify's shortcut when the
    /// button can't be found (an old version, another language).
    async fn press(&self, action: &SpotifyAction) -> Result<String, ToolError> {
        let w = self.window().await?.ok_or_else(|| ToolError::Unavailable("Spotify isn't open".into()))?;
        let q = WindowQuery::id(w.id);
        match action {
            SpotifyAction::Play | SpotifyAction::Pause => {
                let (want, other) = if *action == SpotifyAction::Play {
                    (spotify::play_button(), spotify::pause_button())
                } else {
                    (spotify::pause_button(), spotify::play_button())
                };
                match self.ui.act(&q, &want, &UiAction::Click).await {
                    Ok(_) => return Ok("done".into()),
                    Err(UiError::NotFound(_)) => {}
                    Err(e) => return Err(failed(&e)),
                }
                let snap = self.ui.snapshot(&q).await.map_err(|e| failed(&e))?;
                if other.pick(&snap.elements).is_some() {
                    let already = if *action == SpotifyAction::Play { "already playing" } else { "already paused" };
                    return Ok(already.into());
                }
                Err(ToolError::Failed("I can't find Spotify's play button".into()))
            }
            _ => {
                let control = spotify::control(action).ok_or_else(|| ToolError::BadArguments("not a button".into()))?;
                match self.ui.act(&q, &control.button, &UiAction::Click).await {
                    Ok(_) => Ok("done".into()),
                    Err(UiError::NotFound(what)) => match &control.shortcut {
                        Some(keys) => {
                            tracing::info!(?action, %keys, "button not found, using the shortcut");
                            self.ui.press(&q, keys).await.map(|()| "done".into()).map_err(|e| failed(&e))
                        }
                        None => Err(ToolError::Failed(format!("I can't find {what} in Spotify"))),
                    },
                    Err(e) => Err(failed(&e)),
                }
            }
        }
    }

    /// Opens the search for `query` and clicks the play button of the best result.
    async fn play_search(&self, query: &str) -> Result<String, ToolError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(ToolError::BadArguments("what should I play?".into()));
        }
        let was_open = self.window().await?.is_some();
        self.opener.open(&spotify::search_uri(query)).map_err(ToolError::Failed)?;
        let w = if was_open {
            self.window().await?.ok_or_else(|| ToolError::Failed("Spotify closed".into()))?
        } else {
            self.wait_for_window().await?
        };
        let q = WindowQuery::id(w.id);
        let results = spotify::result_buttons();
        let started = tokio::time::Instant::now();
        let deadline = started + self.patience.results + if was_open { Duration::ZERO } else { self.patience.launch };
        // the previous page may still be on screen for a moment: wait for results that fit the
        // query, and only settle for the top result once the page had time to change
        let settle = started + self.patience.results / 2;
        loop {
            tokio::time::sleep(self.patience.poll).await;
            let snap = match self.ui.snapshot(&q).await {
                Ok(s) => s,
                Err(UiError::NoWindow(_)) => continue,
                Err(e) => return Err(failed(&e)),
            };
            let hits: Vec<usize> =
                snap.elements.iter().enumerate().filter(|(_, e)| results.matches(e)).map(|(i, _)| i).collect();
            let names: Vec<&str> = hits.iter().map(|&i| snap.elements[i].name.as_str()).collect();
            let fits = capri_domain::words::normalize(query)
                .split_whitespace()
                .all(|w| names.iter().any(|n| capri_domain::words::normalize(n).contains(w)));
            let now = tokio::time::Instant::now();
            if !names.is_empty() && (fits || now >= settle) {
                let pick = spotify::best_result(&names, query).map_or(hits[0], |i| hits[i]);
                let target = snap.locator_for(pick).ok_or_else(|| ToolError::Failed("result vanished".into()))?;
                let name = self.ui.act(&q, &target, &UiAction::Click).await.map_err(|e| failed(&e))?;
                return Ok(format!("playing {}", spotify::result_title(&name)));
            }
            if now >= deadline {
                return Err(ToolError::Failed(format!(
                    "I opened the search for \"{query}\" but found no result to play"
                )));
            }
        }
    }

    async fn now_playing(&self) -> Result<String, ToolError> {
        let w = self.window().await?.ok_or_else(|| ToolError::Unavailable("Spotify isn't open".into()))?;
        Ok(spotify::now_playing(&w.title).map_or_else(|| "nothing playing".into(), |s| format!("playing {s}")))
    }

    pub async fn perform(&self, action: &SpotifyAction) -> Result<String, ToolError> {
        tracing::info!(?action, "spotify");
        match action {
            SpotifyAction::Open => self.open().await.map(|_| "Spotify is open".into()),
            SpotifyAction::NowPlaying => self.now_playing().await,
            SpotifyAction::PlayQuery(q) => self.play_search(q).await,
            other => self.press(other).await,
        }
    }
}

#[async_trait]
impl Tool for SpotifyRemote {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: SPOTIFY_TOOL.into(),
            description: "Drive the Spotify desktop app by pressing its buttons: play, pause, toggle, next, \
                previous, shuffle, repeat, like (add the current song to Liked Songs), play_query (search \
                for `query`, a song, artist, album or playlist, and start the best match), now_playing \
                (what song is on), open."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["play", "pause", "toggle", "next", "previous", "shuffle", "repeat", "like", "play_query", "now_playing", "open"] },
                    "query": { "type": "string", "description": "for play_query: what to play, e.g. 'Get Lucky Daft Punk'" }
                },
                "required": ["action"]
            }),
            risk: Risk::Reversible,
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Risk {
        if args.get("action").and_then(|a| a.as_str()) == Some("now_playing") { Risk::Read } else { Risk::Reversible }
    }

    fn describe(&self, args: &serde_json::Value) -> String {
        match serde_json::from_value::<SpotifyAction>(args.clone()) {
            Ok(SpotifyAction::PlayQuery(q)) => format!("Spotify: play \"{q}\""),
            Ok(a) => format!("Spotify: {}", format!("{a:?}").to_lowercase()),
            Err(_) => format!("Spotify {args}"),
        }
    }

    async fn run(&self, args: serde_json::Value) -> Result<String, ToolError> {
        let action: SpotifyAction = serde_json::from_value(args).map_err(|e| ToolError::BadArguments(e.to_string()))?;
        self.perform(&action).await
    }
}
