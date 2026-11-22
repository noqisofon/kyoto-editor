//! kyoto: a terminal text editor configured with the Koto language.

mod app;
mod input;
mod ui;

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use crossterm::event::{
    self, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::terminal::supports_keyboard_enhancement;
use crossterm::{execute, event::DisableBracketedPaste, event::EnableBracketedPaste};
use kyoto_core::config::{self, Loaded};
use kyoto_core::{Command, Editor, KeyMap};

use app::App;

const HELP: &str = "\
kyoto - a text editor configured in Koto

USAGE:
    kyoto [OPTIONS] [FILE]

OPTIONS:
    -c, --config <PATH>   Use this config file (default: $KYOTO_CONFIG or
                          ~/.config/kyoto/config.koto)
        --print-config    Print an example config to stdout
        --list-commands   List command names usable in keybindings
        --list-keys       List the default CUA key bindings
    -h, --help            Show this help
    -V, --version         Show version
";

struct Cli {
    file: Option<PathBuf>,
    config: Option<PathBuf>,
}

fn parse_args() -> Result<Option<Cli>, String> {
    let mut cli = Cli { file: None, config: None };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("kyoto {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--print-config" => {
                print!("{}", config::DEFAULT_CONFIG);
                return Ok(None);
            }
            "--list-commands" => {
                for n in Command::all_names() {
                    println!("{n}");
                }
                return Ok(None);
            }
            "--list-keys" => {
                for (k, b) in KeyMap::cua().bindings() {
                    if let kyoto_core::Binding::Command(c) = b {
                        println!("{k:<18} {}", c.name());
                    }
                }
                return Ok(None);
            }
            "-c" | "--config" => {
                let p = args.next().ok_or("--config needs a path")?;
                cli.config = Some(PathBuf::from(p));
            }
            "--" => {
                cli.file = args.next().map(PathBuf::from);
                break;
            }
            s if s.starts_with('-') && s.len() > 1 => return Err(format!("unknown option `{s}`")),
            _ => {
                if cli.file.is_some() {
                    return Err("only one file can be opened".into());
                }
                cli.file = Some(PathBuf::from(a));
            }
        }
    }
    Ok(Some(cli))
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn run(mut app: App) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let mut out = io::stdout();
    let _ = execute!(out, EnableBracketedPaste);
    let enhanced = supports_keyboard_enhancement().unwrap_or(false);
    if enhanced {
        let _ = execute!(
            out,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }

    let result = (|| -> io::Result<()> {
        while !app.should_quit {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    if let Some(key) = input::convert(k) {
                        app.handle_key(key);
                    }
                }
                Event::Paste(s) => app.handle_paste(&s),
                _ => {}
            }
            for text in app.clipboard_out.drain(..) {
                // OSC 52: ask the terminal to put the text on the system clipboard.
                write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()))?;
                out.flush()?;
            }
        }
        Ok(())
    })();

    if enhanced {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(out, DisableBracketedPaste);
    ratatui::restore();
    result
}

fn main() -> ExitCode {
    let cli = match parse_args() {
        Ok(Some(c)) => c,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kyoto: {e}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };

    let config_path = cli.config.clone().or_else(config::default_config_path);
    let loaded = match &config_path {
        Some(p) => match config::load_path(p) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("kyoto: error in config {}:\n{e}", p.display());
                return ExitCode::FAILURE;
            }
        },
        None => Loaded::defaults(),
    };

    let editor = match &cli.file {
        Some(p) => match Editor::open(p) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("kyoto: cannot open {}: {e}", p.display());
                return ExitCode::FAILURE;
            }
        },
        None => Editor::new(),
    };

    match run(App::new(editor, loaded, config_path)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kyoto: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64("あ".as_bytes()), "44GC");
    }
}
