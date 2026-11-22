//! Configuration written in the Koto language.
//!
//! The config file is an ordinary Koto script. Whatever it `export`s is read
//! as settings:
//!
//! ```koto
//! export
//!   tab_width: 2
//!   keybindings:
//!     'ctrl+d': 'duplicate_line'
//!     'ctrl+q': null                  # unbind
//!     'f2': |ctx| ctx.selection.to_uppercase()
//! ```
//!
//! Functions bound to keys receive a context map (`selection`, `line`,
//! `line_number`, `column`, `path`) and may return a string, which replaces
//! the selection, or `null` to do nothing.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use koto::prelude::*;

use crate::color::{Color, Theme};
use crate::command::Command;
use crate::editor::{EditOptions, ScriptContext};
use crate::keys::{Binding, Key, KeyMap};

/// The commented example config, also used by `kyoto --print-config`.
pub const DEFAULT_CONFIG: &str = include_str!("default_config.koto");

#[derive(Debug)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError(msg.into()))
}

#[derive(Clone, Debug)]
pub struct Config {
    pub edit: EditOptions,
    pub line_numbers: bool,
    pub theme: Theme,
    pub keymap: KeyMap,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            edit: EditOptions::default(),
            line_numbers: true,
            theme: Theme::default(),
            keymap: KeyMap::cua(),
        }
    }
}

/// Holds the Koto runtime that owns functions bound to keys.
pub struct Script {
    koto: Koto,
    functions: Vec<KValue>,
}

impl Script {
    fn empty() -> Self {
        Self { koto: Koto::default(), functions: Vec::new() }
    }

    /// Calls a bound function. `Ok(Some(text))` means "insert this text".
    pub fn call(&mut self, index: usize, ctx: &ScriptContext) -> Result<Option<String>, ConfigError> {
        let Some(f) = self.functions.get(index).cloned() else {
            return err(format!("no script function #{index}"));
        };
        let map = KMap::new();
        map.insert("selection", KValue::Str(ctx.selection.as_str().into()));
        map.insert("line", KValue::Str(ctx.line.as_str().into()));
        map.insert("line_number", KValue::Number((ctx.line_number as i64).into()));
        map.insert("column", KValue::Number((ctx.column as i64).into()));
        map.insert(
            "path",
            match &ctx.path {
                Some(p) => KValue::Str(p.as_str().into()),
                None => KValue::Null,
            },
        );
        match self.koto.call_function(f, KValue::Map(map)) {
            Ok(KValue::Null) => Ok(None),
            Ok(KValue::Str(s)) => Ok(Some(s.to_string())),
            Ok(other) => err(format!(
                "key function must return a string or null, got {}",
                other.type_as_string()
            )),
            Err(e) => err(format!("script error: {e}")),
        }
    }
}

pub struct Loaded {
    pub config: Config,
    pub script: Script,
    /// Non-fatal problems (unknown keys, bad key strings, ...).
    pub warnings: Vec<String>,
}

impl Loaded {
    pub fn defaults() -> Self {
        Self { config: Config::default(), script: Script::empty(), warnings: Vec::new() }
    }
}

/// `$KYOTO_CONFIG`, else `$XDG_CONFIG_HOME/kyoto/config.koto`, else
/// `~/.config/kyoto/config.koto`.
pub fn default_config_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KYOTO_CONFIG").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("kyoto").join("config.koto"))
}

/// Loads a config file. A missing file is not an error: defaults are used.
pub fn load_path(path: &Path) -> Result<Loaded, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(src) => load_str(&src, Some(path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Loaded::defaults()),
        Err(e) => err(format!("cannot read {}: {e}", path.display())),
    }
}

pub fn load_str(src: &str, path: Option<&Path>) -> Result<Loaded, ConfigError> {
    let settings = KotoSettings::default().with_execution_limit(Duration::from_secs(2));
    let mut koto = Koto::with_settings(settings);

    let api = KMap::new();
    api.insert("version", KValue::Str(env!("CARGO_PKG_VERSION").into()));
    let names: Vec<KValue> = Command::all_names().iter().map(|n| KValue::Str(n.as_str().into())).collect();
    api.insert("commands", KValue::Tuple(names.into()));
    koto.prelude().insert("kyoto", api);

    let args = match path {
        Some(p) => CompileArgs::new(src).script_path(p.display().to_string()),
        None => CompileArgs::new(src),
    };
    if let Err(e) = koto.compile_and_run(args) {
        return err(format!("{e}"));
    }

    let exports = koto.exports().clone();
    let mut cfg = Config::default();
    let mut warnings = Vec::new();
    let mut functions = Vec::new();

    let entries: Vec<(String, KValue)> = exports
        .data()
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();

    for (key, value) in entries {
        match key.as_str() {
            "tab_width" => {
                let n = int(&key, &value)?;
                if !(1..=16).contains(&n) {
                    return err("`tab_width` must be between 1 and 16");
                }
                cfg.edit.tab_width = n as usize;
            }
            "insert_spaces" => cfg.edit.insert_spaces = boolean(&key, &value)?,
            "auto_indent" => cfg.edit.auto_indent = boolean(&key, &value)?,
            "line_numbers" => cfg.line_numbers = boolean(&key, &value)?,
            "theme" => load_theme(&value, &mut cfg.theme, &mut warnings)?,
            "keybindings" => {
                load_keybindings(&value, &mut cfg.keymap, &mut functions, &mut warnings)?
            }
            other => warnings.push(format!("unknown setting `{other}` ignored")),
        }
    }

    Ok(Loaded { config: cfg, script: Script { koto, functions }, warnings })
}

fn int(key: &str, v: &KValue) -> Result<i64, ConfigError> {
    match v {
        KValue::Number(n) if n.is_i64() => Ok(i64::from(*n)),
        _ => err(format!("`{key}` must be an integer")),
    }
}

fn boolean(key: &str, v: &KValue) -> Result<bool, ConfigError> {
    match v {
        KValue::Bool(b) => Ok(*b),
        _ => err(format!("`{key}` must be true or false")),
    }
}

fn load_theme(v: &KValue, theme: &mut Theme, warnings: &mut Vec<String>) -> Result<(), ConfigError> {
    let KValue::Map(map) = v else {
        return err("`theme` must be a map");
    };
    let entries: Vec<(String, KValue)> =
        map.data().iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    for (key, value) in entries {
        let KValue::Str(s) = &value else {
            return err(format!("theme.{key} must be a color string"));
        };
        let color = Color::parse(s).map_err(|e| ConfigError(format!("theme.{key}: {e}")))?;
        match key.as_str() {
            "foreground" => theme.foreground = Some(color),
            "background" => theme.background = Some(color),
            "selection_fg" => theme.selection_fg = Some(color),
            "selection_bg" => theme.selection_bg = color,
            "current_line_bg" => theme.current_line_bg = Some(color),
            "line_number" => theme.line_number = color,
            "current_line_number" => theme.current_line_number = color,
            "status_fg" => theme.status_fg = color,
            "status_bg" => theme.status_bg = color,
            other => warnings.push(format!("unknown theme color `{other}` ignored")),
        }
    }
    Ok(())
}

fn load_keybindings(
    v: &KValue,
    keymap: &mut KeyMap,
    functions: &mut Vec<KValue>,
    warnings: &mut Vec<String>,
) -> Result<(), ConfigError> {
    let KValue::Map(map) = v else {
        return err("`keybindings` must be a map of key -> command");
    };
    let entries: Vec<(String, KValue)> =
        map.data().iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    for (key_str, value) in entries {
        let key = match Key::parse(&key_str) {
            Ok(k) => k,
            Err(e) => {
                warnings.push(format!("keybindings: {e}"));
                continue;
            }
        };
        match &value {
            KValue::Null => keymap.unbind(key),
            KValue::Str(name) => match Command::from_name(name) {
                Some(cmd) => keymap.bind(key, Binding::Command(cmd)),
                None => warnings.push(format!("keybindings: unknown command `{name}` for `{key_str}`")),
            },
            f if f.is_callable() => {
                functions.push(f.clone());
                keymap.bind(key, Binding::Script(functions.len() - 1));
            }
            other => warnings.push(format!(
                "keybindings: `{key_str}` must be a command name, a function or null, got {}",
                other.type_as_string()
            )),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Key;

    fn lookup(l: &Loaded, k: &str) -> Option<Binding> {
        l.config.keymap.lookup(Key::parse(k).unwrap())
    }

    #[test]
    fn bundled_default_config_loads_cleanly() {
        let l = load_str(DEFAULT_CONFIG, None).unwrap();
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
        assert_eq!(lookup(&l, "ctrl+c"), Some(Binding::Command(Command::Copy)));
    }

    #[test]
    fn settings_and_bindings() {
        let l = load_str(
            "
export
  tab_width: 2
  insert_spaces: false
  theme:
    background: '#101010'
    selection_bg: 'blue'
  keybindings:
    'ctrl+d': 'duplicate_line'
    'ctrl+q': null
    'f2': |ctx| ctx.selection.to_uppercase()
    'f4': 'no_such_command'
",
            None,
        )
        .unwrap();
        assert_eq!(l.config.edit.tab_width, 2);
        assert!(!l.config.edit.insert_spaces);
        assert_eq!(l.config.theme.background, Some(Color::Rgb(16, 16, 16)));
        assert_eq!(lookup(&l, "ctrl+d"), Some(Binding::Command(Command::DuplicateLine)));
        assert_eq!(lookup(&l, "ctrl+q"), None);
        assert_eq!(l.warnings.len(), 1);
        assert!(matches!(lookup(&l, "f2"), Some(Binding::Script(0))));
    }

    #[test]
    fn script_function_runs() {
        let mut l = load_str(
            "export\n  keybindings:\n    'f2': |ctx| ctx.selection.to_uppercase()\n    'f3': |_| null\n",
            None,
        )
        .unwrap();
        let ctx = ScriptContext {
            selection: "abc".into(),
            line: "abc".into(),
            line_number: 1,
            column: 1,
            path: None,
        };
        assert_eq!(l.script.call(0, &ctx).unwrap().as_deref(), Some("ABC"));
        assert_eq!(l.script.call(1, &ctx).unwrap(), None);
    }

    #[test]
    fn errors_are_reported() {
        assert!(load_str("export\n  tab_width: 'x'\n", None).is_err());
        assert!(load_str("export\n  tab_width: 0\n", None).is_err());
        assert!(load_str("this is not koto (", None).is_err());
        let l = load_str("export\n  mystery: 1\n", None).unwrap();
        assert_eq!(l.warnings.len(), 1);
    }

    #[test]
    fn runaway_script_is_stopped() {
        assert!(load_str("loop\n  x = 1", None).is_err());
    }
}
