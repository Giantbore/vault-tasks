---
title: README
date created: 2026-11-07T12:57
date modified: 2026-11-07T20:05
---
# Vault Tasks
A small native macOS app that collects every open task from your Obsidian vaults into one list. It reads and writes the same format as the Obsidian [Tasks](https://github.com/obsidian-tasks-group/obsidian-tasks) plugin, so everything stays in your notes.

## Features
- **All** view groups open tasks by due date: Overdue, Today, Next 7 Days, Later, No Due Date.
- **Starred** view shows tasks you star. Starring sets the task's ⏳ scheduled date to today, so it also shows as scheduled in Obsidian.
- Per-vault views in the sidebar, plus Quick Find search.
- Check a task off to mark it done in its note (`[x]` and `✅ date`), with an optional completion chime and an Undo bar.
- **Completed** view lists tasks done today, in the past 7 days or past 30 days, grouped by day. Uncheck one to reopen it.
- Click a date chip (or the 📅 button on undated tasks) to set, change or remove a due date. Quick picks plus a calendar; the `📅` date is written to the note.
- Click a task's name to rename it in place (dates, priority and other metadata are kept). Long names wrap.
- Click the grey vault · file line under a task to open its note in Obsidian. With the free [Advanced URI](https://github.com/Vinzent03/obsidian-advanced-uri) plugin enabled in a vault, the note opens at the task's line.
- **Notes**: click the ≡ button on a task to add Markdown notes (bullets, bold, italic, code, headings, links).
- **Lists**: create custom lists under LISTS in the sidebar (right-click a list to rename or delete it). Add tasks from a task's notes panel or by right-clicking its name. New tasks made inside a list view join that list.
- Notes and lists are stored in `.vault-tasks.json` at each vault's root, so they sync with the vault and don't clutter your notes. A task gets an Obsidian block ID (`^vt-abc123`) at the end of its line the first time you give it notes or a list, so the app can find it again after edits.
- Create tasks from the app. They are added to `Vault Tasks.md` at the root of the chosen vault.
- Light and dark themes (or match macOS), with a matching logo and Dock icon.
- Settings for vault folders, excluded folders, default due date, chime sound, appearance, font and text size.
- Native macOS menu bar with keyboard shortcuts.

## Download
1. Download the latest **Vault-Tasks-…-macOS.zip** from [Releases](https://github.com/thomaspeterson2/vault-tasks/releases/latest) and unzip it.
2. Drag **Vault Tasks.app** into your Applications folder.
3. Open it. macOS will say it can't verify the developer, because the app isn't signed with an Apple Developer ID.
4. Open **System Settings → Privacy & Security**, scroll down, and click **Open Anyway** next to Vault Tasks. You only need to do this once.

Works on Apple Silicon and Intel Macs running macOS 11 or later.

## Build from Source
Requirements: macOS 11 or later and the Rust toolchain.
```bash
brew install rustup
rustup default stable
```
Build and install the app into `~/Applications`:
```bash
./bundle.sh --install
```
Other options: `--universal` builds one app for Apple Silicon and Intel, and `--zip` writes a shareable zip to `target/`. Builds go to `target/bundle.noindex/` so Spotlight doesn't list them as extra apps.

## Publishing a Release
1. Bump `version` in `Cargo.toml` and commit.
2. Tag and push: `git tag v0.2.0 && git push origin main v0.2.0`.
3. The **Release** GitHub Action runs the tests, builds the universal app and attaches the zip to a new release.

## First Run
The app looks for vaults in Obsidian's iCloud folder (`~/Library/Mobile Documents/iCloud~md~obsidian/Documents`). Vaults stored anywhere else can be added in **Settings → Vault folders**. Settings are saved to `~/Library/Application Support/vault-tasks/config.json`.

## Task Format
Tasks are Markdown checkboxes. These Tasks plugin markers are recognized:
- `📅 2026-11-10` due date
- `⏳ 2026-11-10` scheduled date (used for the Today list)
- `🛫 2026-11-10` start date
- `➕ 2026-11-10` created date
- `✅ 2026-11-10` done date
- `🔺 ⏫ 🔼 🔽 ⏬` priority
- `🔁 every week` recurrence

Recurring tasks open in Obsidian when checked, so the Tasks plugin can create the next occurrence. Tasks inside code blocks and hidden folders such as `.obsidian` are ignored.

## Keyboard Shortcuts
- ⌘N new task
- ⌘R refresh
- ⌘1 All, ⌘2 Today, ⌘3 Completed
- ⌘, Settings
- ⌘= bigger text, ⌘- smaller text, ⌘0 normal text

## Custom Chime Sounds
The chime picker lists the macOS system sounds plus anything in `~/Library/Sounds`. Drop `.aiff`, `.wav`, `.mp3`, `.m4a` or `.caf` files there (Settings has an **Open folder** button). Short clips under one second work best.

To make your own, record in Voice Memos or QuickTime, or download a free clip, then trim it and convert it with the built-in `afconvert`:
```bash
afconvert -f AIFF -d BEI16 input.m4a ~/Library/Sounds/MyDing.aiff
```

## Development
```bash
cargo run      # run a debug build
cargo test     # run unit tests
```
- `src/tasks.rs` scans vaults, parses tasks and edits task lines
- `src/main.rs` contains the app window, views and settings
- `src/menu.rs` builds the macOS menu bar
- `src/fonts.rs` loads the bundled fonts
- `assets/icon.svg` and `assets/icon-light.svg` are the app icon (dark and light); `assets/glyph.svg` and `assets/glyph-light.svg` are the in-app logo without the tile. After editing any of them, run `cargo run --example make_icon` to regenerate the PNGs and `assets/AppIcon.icns`

## Credits
Built with [egui](https://github.com/emilk/egui). Bundled fonts are Manrope and Source Serif 4, both under the SIL Open Font License (see `assets/fonts/README.md`).
