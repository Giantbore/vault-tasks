# Vault Tasks
A small native macOS app that collects every open task from your Obsidian vaults into one list. It reads and writes the same format as the Obsidian [Tasks](https://github.com/obsidian-tasks-group/obsidian-tasks) plugin, so everything stays in your notes.

## Features
- **All** view groups open tasks by due date: Overdue, Today, Next 7 Days, Later, No Due Date.
- **Today** view shows tasks you star. Starring sets the task's ⏳ scheduled date to today, so it also shows as scheduled in Obsidian.
- Per-vault views in the sidebar, plus Quick Find search.
- Check a task off to mark it done in its note (`[x]` and `✅ date`), with an optional completion chime and an Undo bar.
- **Completed** view lists tasks done today, in the past 7 days or past 30 days, grouped by day. Uncheck one to reopen it.
- Click a task to open its note in Obsidian.
- Create tasks from the app. They are added to `Vault Tasks.md` at the root of the chosen vault.
- Settings for vault folders, excluded folders, default due date, chime sound, font and text size.
- Native macOS menu bar with keyboard shortcuts.

## Install
Requirements: macOS 11 or later and the Rust toolchain.
```bash
brew install rustup
rustup default stable
```
Build and install the app into `~/Applications`:
```bash
./bundle.sh --install
```
Run `./bundle.sh` without `--install` to build `target/bundle.noindex/Vault Tasks.app` only (the `.noindex` folder keeps Spotlight from listing build copies). Run the same command again after pulling changes to update the app.

## First run
The app looks for vaults in Obsidian's iCloud folder (`~/Library/Mobile Documents/iCloud~md~obsidian/Documents`). Vaults stored anywhere else can be added in **Settings → Vault folders**. Settings are saved to `~/Library/Application Support/vault-tasks/config.json`.

## Task format
Tasks are Markdown checkboxes. These Tasks plugin markers are recognized:
- `📅 2026-11-10` due date
- `⏳ 2026-11-10` scheduled date (used for the Today list)
- `🛫 2026-11-10` start date
- `➕ 2026-11-10` created date
- `✅ 2026-11-10` done date
- `🔺 ⏫ 🔼 🔽 ⏬` priority
- `🔁 every week` recurrence

Recurring tasks open in Obsidian when checked, so the Tasks plugin can create the next occurrence. Tasks inside code blocks and hidden folders such as `.obsidian` are ignored.

## Keyboard shortcuts
- ⌘N new task
- ⌘R refresh
- ⌘1 All, ⌘2 Today, ⌘3 Completed
- ⌘, Settings
- ⌘= bigger text, ⌘- smaller text, ⌘0 normal text

## Custom chime sounds
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
- `assets/icon.svg` is the app icon source. After editing it, run `cargo run --example make_icon` to regenerate `assets/AppIcon.icns`

## Credits
Built with [egui](https://github.com/emilk/egui). Bundled fonts are Manrope and Source Serif 4, both under the SIL Open Font License (see `assets/fonts/README.md`).
