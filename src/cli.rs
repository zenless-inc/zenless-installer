//! Command-line parsing and the silent (no UI) install / uninstall modes.

use crate::components::{self, Component};
use crate::install::{self, InstallPlan, Installed, Options};
use crate::platform::Env;
use crate::report::{ConsoleReporter, Reporter};
use crate::uninstall::{self, UninstallPlan};
use std::path::PathBuf;

pub const USAGE: &str = "\
Zenless Setup — installs Zenless Download Manager, Zenless Torrent and the browser extensions.

USAGE:
  ZenlessSetup.exe                         open the setup wizard
  ZenlessSetup.exe --uninstall             open the uninstaller
  ZenlessSetup.exe --silent [OPTIONS]      install without any window
  ZenlessSetup.exe --uninstall --silent [--purge]

OPTIONS (silent install):
  --components dm,torrent,chrome,firefox   what to install (default: everything, or what is
                                           already installed when updating)
  --dir PATH                               install folder (default %LOCALAPPDATA%\\Programs\\Zenless)
  --no-shortcuts                           no Start menu or desktop shortcuts
  --no-desktop                             Start menu shortcuts only
  --no-autostart                           don't start the Download Manager with Windows
  --autostart-torrent                      also start Zenless Torrent with Windows
  --no-associate                           don't handle magnet links / .torrent files
  --theme NAME                             theme for all Zenless apps (e.g. \"Paper\")
  --log PATH                               also append the log to a file
  --purge                                  (uninstall) also remove settings and download history

EXIT CODES: 0 ok, 1 failed (changes rolled back), 2 bad arguments, 3 not installed.
";

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_NOT_INSTALLED: i32 = 3;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args {
    pub uninstall: bool,
    pub silent: bool,
    pub help: bool,
    pub components: Option<Vec<Component>>,
    pub dir: Option<PathBuf>,
    pub no_shortcuts: bool,
    pub no_desktop: bool,
    pub no_autostart: bool,
    pub autostart_torrent: bool,
    pub no_associate: bool,
    pub theme: Option<String>,
    pub log: Option<PathBuf>,
    pub purge: bool,
}

/// Parses the arguments (without the program name).
pub fn parse<I, S>(args: I) -> Result<Args, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut out = Args::default();
    let mut it = args.into_iter().map(Into::into).peekable();
    while let Some(arg) = it.next() {
        // Accept --flag=value as well as --flag value.
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_owned(), Some(v.to_owned())),
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            match inline.clone().or_else(|| it.next()) {
                Some(v) if !v.is_empty() => Ok(v),
                _ => Err(format!("{name} needs a value")),
            }
        };
        match flag.to_ascii_lowercase().as_str() {
            "--uninstall" | "/uninstall" => out.uninstall = true,
            "--silent" | "/silent" | "/s" | "-s" | "--quiet" => out.silent = true,
            "--help" | "-h" | "/?" => out.help = true,
            "--components" => {
                let list = components::parse_list(&value("--components")?)?;
                if list.is_empty() {
                    return Err("--components needs at least one of dm, torrent, chrome, firefox".into());
                }
                out.components = Some(list);
            }
            "--dir" => out.dir = Some(PathBuf::from(value("--dir")?)),
            "--no-shortcuts" => out.no_shortcuts = true,
            "--no-desktop" => out.no_desktop = true,
            "--no-autostart" => out.no_autostart = true,
            "--autostart-torrent" => out.autostart_torrent = true,
            "--no-associate" => out.no_associate = true,
            "--theme" => out.theme = Some(value("--theme")?),
            "--log" => out.log = Some(PathBuf::from(value("--log")?)),
            "--purge" => out.purge = true,
            other => return Err(format!("unknown argument {other:?} (try --help)")),
        }
    }
    Ok(out)
}

/// Builds the install plan for `--silent`.
pub fn silent_install_plan(env: &Env, args: &Args) -> InstallPlan {
    let installed = Installed::detect(env);
    let root = args
        .dir
        .clone()
        .map(|d| std::path::absolute(&d).unwrap_or(d))
        .or_else(|| installed.as_ref().map(|i| i.root.clone()))
        .unwrap_or_else(|| env.default_install_dir());
    let components = args.components.clone().unwrap_or_else(|| match &installed {
        Some(i) if !i.components.is_empty() => i.components.clone(),
        _ => Component::ALL.to_vec(),
    });
    let previous = installed.as_ref().map(|i| i.options.clone());
    let options = Options {
        start_menu: !args.no_shortcuts,
        desktop: !args.no_shortcuts && !args.no_desktop && previous.as_ref().is_none_or(|p| p.desktop),
        autostart_dm: !args.no_autostart,
        autostart_torrent: args.autostart_torrent || previous.as_ref().is_some_and(|p| p.autostart_torrent),
        associate: !args.no_associate,
    };
    InstallPlan { root, components, remove: Vec::new(), options, theme: args.theme.clone() }
}

/// Runs a silent install or uninstall and returns the process exit code.
pub fn run_silent(env: &Env, args: &Args) -> i32 {
    let reporter = ConsoleReporter::new(args.log.as_deref());
    if args.uninstall {
        let Some(installed) = Installed::detect(env).or_else(|| {
            let me = std::env::current_exe().ok()?;
            let root = me.parent()?.to_path_buf();
            Installed::from_folder(env, root)
        }) else {
            reporter.error("Zenless Suite is not installed.");
            return EXIT_NOT_INSTALLED;
        };
        let plan = UninstallPlan {
            root: installed.root.clone(),
            components: args.components.clone().unwrap_or(installed.components.clone()),
            remove_settings: args.purge,
        };
        match uninstall::run(env, &plan, &reporter) {
            Ok(outcome) => {
                if let Some(cmd) = outcome.self_delete
                    && let Err(e) = crate::platform::spawn_hidden_cmd(&cmd)
                {
                    reporter.warn(&format!("Could not schedule removal of the install folder: {e}"));
                }
                EXIT_OK
            }
            Err(e) => {
                reporter.error(&e);
                EXIT_FAILED
            }
        }
    } else {
        let plan = silent_install_plan(env, args);
        if let Err(e) = install::validate_root(&plan.root) {
            reporter.error(&e);
            return EXIT_USAGE;
        }
        match install::run(env, &plan, &reporter) {
            Ok(outcome) => {
                if outcome.warnings > 0 {
                    reporter.warn(&format!("Finished with {} warning(s) while cleaning up.", outcome.warnings));
                }
                EXIT_OK
            }
            Err(_) => EXIT_FAILED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_means_wizard() {
        assert_eq!(parse(Vec::<String>::new()).unwrap(), Args::default());
    }

    #[test]
    fn silent_install_flags() {
        let a = parse(["--silent", "--components", "dm,firefox", "--dir", r"D:\Apps\Zenless", "--no-shortcuts"]).unwrap();
        assert!(a.silent && !a.uninstall && a.no_shortcuts);
        assert_eq!(a.components, Some(vec![Component::Dm, Component::Firefox]));
        assert_eq!(a.dir, Some(PathBuf::from(r"D:\Apps\Zenless")));

        let b = parse(["/S", "--components=torrent", "--dir=C:\\Z", "--theme", "Paper", "--log", "x.log"]).unwrap();
        assert!(b.silent);
        assert_eq!(b.components, Some(vec![Component::Torrent]));
        assert_eq!(b.dir, Some(PathBuf::from("C:\\Z")));
        assert_eq!(b.theme.as_deref(), Some("Paper"));
        assert_eq!(b.log, Some(PathBuf::from("x.log")));
    }

    #[test]
    fn uninstall_flags() {
        let a = parse(["--uninstall", "--silent", "--purge"]).unwrap();
        assert!(a.uninstall && a.silent && a.purge);
        assert!(parse(["--uninstall"]).unwrap().uninstall);
    }

    #[test]
    fn bad_arguments() {
        assert!(parse(["--components"]).is_err());
        assert!(parse(["--components", "dm,opera"]).is_err());
        assert!(parse(["--components="]).is_err());
        assert!(parse(["--dir"]).is_err());
        assert!(parse(["--frobnicate"]).is_err());
        assert!(parse(["--help"]).unwrap().help);
    }

    #[test]
    fn silent_plan_defaults() {
        let sb = std::env::temp_dir().join(format!("zenless-cli-plan-{}", std::process::id()));
        let env = Env::with_sandbox(Some(sb.clone()));
        let plan = silent_install_plan(&env, &parse(["--silent"]).unwrap());
        assert_eq!(plan.root, sb.join("install"));
        assert_eq!(plan.components, Component::ALL.to_vec());
        assert!(plan.options.start_menu && plan.options.desktop && plan.options.associate);

        let plan = silent_install_plan(&env, &parse(["--silent", "--no-shortcuts", "--components", "chrome"]).unwrap());
        assert!(!plan.options.start_menu && !plan.options.desktop);
        assert_eq!(plan.components, vec![Component::Chrome]);
    }
}
