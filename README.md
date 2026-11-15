<h1 align="center">capri-rs</h1>

<p align="center">
  <img src="docs/media/hero.gif" alt="Capri sitting on the edge of a terminal window, swinging her legs" width="860">
</p>

<p align="center">
  A netrunner who lives on your desktop. She sits on your windows, falls when you close them,<br>
  naps when you ignore her, and talks back through whatever model you point her at.
</p>

---

capri-rs is a Rust rewrite of [capri-ai-desktop](https://github.com/edgerunners-2/capri-ai-desktop).
Same character, same sprite sheets, new engine: one native binary, a ports-and-adapters core you can
test without a screen, and a brain that can be a local llama.cpp, Ollama, LM Studio, a cloud API, or
Claude Code.

Capri is drawn by **xilia_world00** and is a fan take on Capricorn from *Cyberpunk: Edgerunners*.
More on that in [Credits](#credits).

## Run her

You need Rust (stable) and something that speaks the OpenAI chat API. A llama.cpp server is enough:

```sh
llama-server -m your-model.gguf --port 8080
```

Then:

```sh
git clone https://github.com/Rhombicdodecahedron/capri-rs
cd capri-rs
cargo run --release
```

She drops in from the top of the screen and lands on the first window edge she meets. Right-click her
to open the settings and point her at your server.

Windows is the main target (real per-pixel transparency, clicks go through the empty parts of her
window). Linux and macOS run, with her standing on the bottom of the screen until those platforms get
their own window-edge adapter.

## Talk to her

<img src="docs/media/talk.gif" alt="Capri talking, with her reply in a cyberpunk chat box" width="420" align="right">

Click her and type. Enter sends, Esc closes. The reply streams in while her mouth moves.

She answers in the language you write in. The labels around her follow the interface language
(French or English, or whatever your system uses).

A few things work without a round trip to the model:

- `souviens-toi que ...` / `remember that ...` saves a fact about you
- `oublie ...` / `forget ...` drops it
- `/new` starts a fresh chat
- `/settings` opens the settings, `/quit` sends her home

Drag her to move her, let go fast to throw her. Double-click to poke her awake.

Facts and the chat log stay on your machine, in a SQLite file in your profile folder.

<br clear="right">

## Settings

<img src="docs/media/settings.png" alt="The settings panel, brain tab, with a successful link test against llama.cpp" width="640">

Right-click her, or type `/settings`. Everything she reads from `capri.toml` is in there: the brain
(engine, URL, key, model, temperature), her body (skin, size, how often she hops around, when she
dozes off), her persona prompt, what she is allowed to do on your PC, and the language.

**Test** lists the models on the server and times a short reply before you save anything. Saving
applies at once, no restart: she swaps brains mid-conversation and reloads her sprites if you changed
the skin or the size.

For llama.cpp, leave the model empty to use the first one the server lists, or pick from the names TEST shows (servers hosting several models only accept those exact names). Reasoning
models (Qwen3 and friends) have their thinking switched off by default, so the first word comes fast;
there is a checkbox if you want it back.

The file is plain TOML if you prefer editing it by hand. `cargo run -- paths` tells you where it is,
and comments you write in it survive saves from the panel.

## Other commands

```sh
cargo run -- chat               # same brain and memory, in the terminal
cargo run -- brain              # test the configured brain from the command line
cargo run -- skins              # list installed skins
cargo run -- preview capri      # render a skin's poses to preview.png
```

## How it is put together

```text
apps/capri                     reads capri.toml, builds the adapters, starts the desk
│
├── adapters/                  everything that touches the outside world
│   ├── desk-winit             her window: winit + tiny-skia, layered window on Windows
│   ├── settings-egui          the settings panel
│   ├── llm-openai             llama.cpp, Ollama, LM Studio, OpenAI, OpenRouter, anything /v1
│   ├── llm-claude-code        the Claude Code CLI
│   ├── desktop-windows        visible window edges via Win32
│   ├── memory-sqlite          facts and chat log
│   └── skin-fs                sprite sheets from assets/skins
│
├── crates/capri-config        the capri.toml schema, comment-preserving save
├── crates/capri-application   use cases (chat, desk, tools) and the ports adapters implement
└── crates/capri-domain        pure logic: falling, perching, blinking, roaming, approvals
```

Dependencies point inward only. The domain has no I/O at all: time comes in as a number, the desktop
comes in as a list of ledges, randomness comes from a seeded generator. That is why falling off a
closed window or dozing off after five idle minutes are plain unit tests.

Anything she does on your PC will go through one approval check in the application layer. Text from
the model can ask for an action, never approve one.

## Skins

A skin is a folder of PNG strips in `assets/skins/<name>/`, in the same format as the original project,
so its skins drop in unchanged. See [assets/skins/README.md](assets/skins/README.md). A placeholder
`demo` skin is included for testing.

## Where it is going

- [x] On the desk: perching, falling, roaming, sleeping, chat, memory
- [x] Brains: llama.cpp, Ollama, LM Studio, OpenAI, OpenRouter, Claude Code
- [x] Settings panel with live reload, French and English
- [ ] Tools (apps, music, files, web) behind the approval check
- [ ] MCP client
- [ ] Voice in and out
- [ ] Mini form and lying-down poses (the art is already in the skin)
- [ ] Window edges on macOS and Linux

## Credits

- **Art:** Capri by xilia_world00 (Discord), used with permission. The art is
  **not** under this repository's MIT license; ask her before reusing it.
- **Original project:** [capri-ai-desktop](https://github.com/edgerunners-2/capri-ai-desktop) by
  yume_mud (MIT), where the idea and the sprite format come from.
- **Font:** Share Tech Mono, SIL Open Font License.
- Capri is a fan character based on Capricorn from *Cyberpunk: Edgerunners*. Cyberpunk is a trademark of
  CD PROJEKT RED. This is an unofficial fan project, not affiliated with or endorsed by CD PROJEKT RED,
  Studio Trigger or Netflix.

Code under the MIT license, see [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md).
