//! Application state: glues the core editor to config, prompts and scrolling.

use std::path::PathBuf;

use kyoto_core::config::{self, Config, Loaded, Script};
use kyoto_core::{Binding, Command, Editor, Effect, Key, KeyCode, PromptKind};

pub struct Prompt {
    pub kind: PromptKind,
    pub input: String,
}

impl Prompt {
    pub fn label(&self) -> &'static str {
        match self.kind {
            PromptKind::Find => "Find",
            PromptKind::GotoLine => "Go to line",
            PromptKind::SaveAs => "Save as",
        }
    }
}

pub struct App {
    pub editor: Editor,
    pub config: Config,
    script: Script,
    config_path: Option<PathBuf>,
    pub prompt: Option<Prompt>,
    pub message: Option<String>,
    quit_armed: bool,
    pub should_quit: bool,
    /// First visible line / display column (maintained by the renderer).
    pub top: usize,
    pub left: usize,
    /// Text queued for the system clipboard (sent as OSC 52).
    pub clipboard_out: Vec<String>,
}

impl App {
    pub fn new(mut editor: Editor, loaded: Loaded, config_path: Option<PathBuf>) -> Self {
        editor.options = loaded.config.edit;
        let message = loaded.warnings.first().map(|w| format!("config: {w}"));
        Self {
            editor,
            config: loaded.config,
            script: loaded.script,
            config_path,
            prompt: None,
            message,
            quit_armed: false,
            should_quit: false,
            top: 0,
            left: 0,
            clipboard_out: Vec::new(),
        }
    }

    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
    }

    pub fn handle_key(&mut self, key: Key) {
        if self.prompt.is_some() {
            self.prompt_key(key);
            return;
        }
        let binding = self.config.keymap.lookup(key);
        if !matches!(binding, Some(Binding::Command(Command::Quit))) {
            self.quit_armed = false;
        }
        self.message = None;
        match binding {
            Some(Binding::Command(cmd)) => self.run(cmd),
            Some(Binding::Script(i)) => self.run_script(i),
            None => {
                if let Some(c) = key.text_char() {
                    self.editor.insert_text(c.encode_utf8(&mut [0; 4]));
                }
            }
        }
        self.collect_editor_output();
    }

    pub fn handle_paste(&mut self, text: &str) {
        if let Some(p) = self.prompt.as_mut() {
            p.input.push_str(&text.replace(['\r', '\n'], " "));
        } else {
            self.quit_armed = false;
            self.editor.insert_text(text);
            self.collect_editor_output();
        }
    }

    fn collect_editor_output(&mut self) {
        if let Some(s) = self.editor.take_status() {
            self.message = Some(s);
        }
        if let Some(c) = self.editor.take_clipboard_export() {
            self.clipboard_out.push(c);
        }
    }

    fn run(&mut self, cmd: Command) {
        match self.editor.execute(cmd) {
            Effect::None => {}
            Effect::Quit => {
                if self.editor.is_modified() && !self.quit_armed {
                    self.quit_armed = true;
                    self.set_message("Unsaved changes! Press the quit key again to discard them.");
                } else {
                    self.should_quit = true;
                }
            }
            Effect::Prompt(kind) => {
                let input = match kind {
                    PromptKind::Find => self
                        .editor
                        .selected_text()
                        .filter(|t| !t.contains('\n'))
                        .or_else(|| self.editor.search_query().map(str::to_string))
                        .unwrap_or_default(),
                    PromptKind::SaveAs => self
                        .editor
                        .path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                    PromptKind::GotoLine => String::new(),
                };
                self.prompt = Some(Prompt { kind, input });
            }
            Effect::ReloadConfig => self.reload_config(),
        }
    }

    fn run_script(&mut self, index: usize) {
        let ctx = self.editor.script_context();
        match self.script.call(index, &ctx) {
            Ok(Some(text)) => self.editor.insert_text(&text),
            Ok(None) => {}
            Err(e) => self.set_message(format!("{e}")),
        }
    }

    pub fn reload_config(&mut self) {
        let Some(path) = self.config_path.clone() else {
            self.set_message("No config path to reload");
            return;
        };
        match config::load_path(&path) {
            Ok(loaded) => {
                let note = match loaded.warnings.first() {
                    Some(w) => format!("Config reloaded (warning: {w})"),
                    None => "Config reloaded".to_string(),
                };
                self.editor.options = loaded.config.edit;
                self.config = loaded.config;
                self.script = loaded.script;
                self.set_message(note);
            }
            Err(e) => self.set_message(format!("Config error: {e}")),
        }
    }

    fn prompt_key(&mut self, key: Key) {
        match key.code {
            KeyCode::Esc => self.prompt = None,
            KeyCode::Enter => {
                let Some(p) = self.prompt.take() else { return };
                self.submit(p);
            }
            KeyCode::Backspace => {
                if let Some(p) = self.prompt.as_mut() {
                    p.input.pop();
                }
            }
            _ => {
                if let (Some(c), Some(p)) = (key.text_char(), self.prompt.as_mut()) {
                    p.input.push(c);
                }
            }
        }
    }

    fn submit(&mut self, p: Prompt) {
        match p.kind {
            PromptKind::Find => {
                self.editor.set_search(&p.input);
                self.editor.find(true);
            }
            PromptKind::GotoLine => match p.input.trim().parse::<usize>() {
                Ok(n) => self.editor.goto_line(n),
                Err(_) => self.set_message("Not a line number"),
            },
            PromptKind::SaveAs => {
                let name = p.input.trim();
                if !name.is_empty() {
                    self.editor.save_as_reporting(name);
                }
            }
        }
        self.collect_editor_output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(text: &str) -> App {
        App::new(Editor::from_text(text), Loaded::defaults(), None)
    }

    fn press(a: &mut App, s: &str) {
        a.handle_key(Key::parse(s).unwrap());
    }

    #[test]
    fn typing_and_cua_flow() {
        let mut a = app("");
        for c in "hello".chars() {
            a.handle_key(Key::new(KeyCode::Char(c), Default::default()));
        }
        press(&mut a, "ctrl+a");
        press(&mut a, "ctrl+c");
        press(&mut a, "end");
        press(&mut a, "ctrl+v");
        assert_eq!(a.editor.text(), "hellohello");
        assert_eq!(a.clipboard_out, vec!["hello".to_string()]);
        press(&mut a, "ctrl+z");
        assert_eq!(a.editor.text(), "hello");
    }

    #[test]
    fn quit_requires_confirmation_when_modified() {
        let mut a = app("");
        a.handle_key(Key::new(KeyCode::Char('x'), Default::default()));
        press(&mut a, "ctrl+q");
        assert!(!a.should_quit);
        press(&mut a, "ctrl+q");
        assert!(a.should_quit);
    }

    #[test]
    fn find_prompt_flow() {
        let mut a = app("one two three");
        press(&mut a, "ctrl+f");
        assert!(a.prompt.is_some());
        for c in "two".chars() {
            a.handle_key(Key::new(KeyCode::Char(c), Default::default()));
        }
        press(&mut a, "enter");
        assert!(a.prompt.is_none());
        assert_eq!(a.editor.selected_text().as_deref(), Some("two"));
    }

    #[test]
    fn script_binding_transforms_selection() {
        let src = "export\n  keybindings:\n    'f2': |ctx| ctx.selection.to_uppercase()\n";
        let loaded = config::load_str(src, None).unwrap();
        let mut a = App::new(Editor::from_text("abc def"), loaded, None);
        press(&mut a, "shift+end");
        press(&mut a, "f2");
        assert_eq!(a.editor.text(), "ABC DEF");
    }
}
