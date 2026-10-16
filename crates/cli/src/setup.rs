//! `chauffeur setup opencode`: point OpenCode at the plugin installed beside
//! this binary, and optionally prepare the env file `brew services` reads.
//!
//! The plugin is linked by its `opt` path, so a package upgrade moves the link
//! with it. Nothing outside OpenCode's plugins folder and the service env file
//! is touched, and an existing file is replaced only with `--force`.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The plugin shim a formula installs beside the daemon.
const SHIM: &str = "share/chauffeur/opencode-plugin.js";
/// The npm package whose plugin would load beside this one.
const NPM_PACKAGE: &str = "@fdcn/chauffeur";

pub fn run(args: &[String]) -> Result<(), String> {
    if args.first().map(String::as_str) != Some("opencode") {
        return Err(usage());
    }
    let mut arguments = args[1..].iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--service" | "--force" | "--dry-run" => {}
            "--plugin" => {
                if arguments.next().is_none_or(|path| path.starts_with("--")) {
                    return Err("--plugin needs a path".into());
                }
            }
            _ => return Err(usage()),
        }
    }

    let flag = |name: &str| args.iter().any(|argument| argument == name);
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set")?;
    let options = Options {
        plugin: match crate::option(args, "--plugin") {
            Some(path) => std::path::absolute(path).map_err(|error| error.to_string())?,
            None => installed_plugin()?,
        },
        opencode: opencode_dir(&home),
        service: flag("--service").then(|| service_env(&home)),
        key: std::env::var("TYPESAFE_API_KEY")
            .ok()
            .filter(|key| !key.is_empty()),
        force: flag("--force"),
        dry_run: flag("--dry-run"),
    };

    for line in setup(&options)? {
        println!("{line}");
    }

    Ok(())
}

fn usage() -> String {
    "usage: chauffeur setup opencode [--plugin PATH] [--service] [--force] [--dry-run]".into()
}

struct Options {
    plugin: PathBuf,
    /// OpenCode's global config folder.
    opencode: PathBuf,
    /// Where `brew services` reads this formula's environment, when asked.
    service: Option<PathBuf>,
    key: Option<String>,
    force: bool,
    dry_run: bool,
}

/// What setup did, or would do with `--dry-run`, one line each.
fn setup(options: &Options) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    let verb = |action: &str| {
        if options.dry_run {
            format!("would {action}")
        } else {
            action.to_string()
        }
    };

    if !options.plugin.is_file() {
        return Err(format!(
            "no plugin at {}; pass --plugin PATH",
            options.plugin.display()
        ));
    }

    if let Some(config) = npm_config(&options.opencode) {
        return Err(format!(
            "{} mentions {NPM_PACKAGE}; remove its plugin entry first to avoid loading Chauffeur twice",
            config.display()
        ));
    }
    if options.service.is_some()
        && options
            .key
            .as_deref()
            .is_some_and(|key| key.contains(['\r', '\n']))
    {
        return Err("TYPESAFE_API_KEY must fit on one line".into());
    }

    let link = options.opencode.join("plugins/chauffeur.js");

    match std::fs::read_link(&link) {
        Ok(target) if target == options.plugin => {
            done.push(format!("{} already links the plugin", link.display()));
        }
        Ok(_) | Err(_) if (link.exists() || link.is_symlink()) && !options.force => {
            return Err(format!(
                "{} already exists; --force replaces it",
                link.display()
            ));
        }
        _ => {
            if !options.dry_run {
                replace_link(&link, &options.plugin)?;
            }
            done.push(verb(&format!(
                "link {} -> {}",
                link.display(),
                options.plugin.display()
            )));
            done.push("restart OpenCode to load it".into());
        }
    }

    if let Some(env) = &options.service {
        done.extend(service(env, options.key.as_deref(), options.dry_run)?);
    }

    Ok(done)
}

fn replace_link(link: &Path, plugin: &Path) -> Result<(), String> {
    let folder = link.parent().ok_or("the plugins folder has no parent")?;

    std::fs::create_dir_all(folder)
        .map_err(|error| format!("create {}: {error}", folder.display()))?;

    if link.exists() || link.is_symlink() {
        std::fs::remove_file(link)
            .map_err(|error| format!("remove {}: {error}", link.display()))?;
    }

    #[cfg(unix)]
    return std::os::unix::fs::symlink(plugin, link)
        .map_err(|error| format!("link {}: {error}", link.display()));

    #[cfg(not(unix))]
    Err("linking the plugin needs a Unix system".into())
}

/// Create the service env file, readable by you alone, with the key from
/// this environment or a line to fill in. An existing file keeps its
/// contents and is made private.
fn service(env: &Path, key: Option<&str>, dry_run: bool) -> Result<Vec<String>, String> {
    let shown = env.display();
    let mut done = Vec::new();

    if env.is_symlink() {
        return Err(format!("{shown} is a symlink; manage that file yourself"));
    }
    if env.exists() {
        if !dry_run {
            private(env)?;
        }
        done.push(format!("{shown} exists; kept it"));
    } else {
        let line = key.map_or_else(
            || "# TYPESAFE_API_KEY=your key\n".to_string(),
            |key| format!("TYPESAFE_API_KEY={key}\n"),
        );

        if !dry_run {
            let folder = env.parent().ok_or("the env file has no parent")?;

            std::fs::create_dir_all(folder)
                .map_err(|error| format!("create {}: {error}", folder.display()))?;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options
                .open(env)
                .map_err(|error| format!("create {shown}: {error}"))?;
            file.write_all(line.as_bytes())
                .map_err(|error| format!("write {shown}: {error}"))?;
        }

        let wrote = if dry_run { "would write" } else { "wrote" };

        done.push(match key {
            Some(_) => format!("{wrote} {shown} with TYPESAFE_API_KEY from this shell"),
            None => format!("{wrote} {shown}; add TYPESAFE_API_KEY to it"),
        });
    }

    done.push("then: brew services start chauffeur".into());

    Ok(done)
}

#[cfg(unix)]
fn private(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("make {} private: {error}", path.display()))
}

#[cfg(not(unix))]
fn private(_: &Path) -> Result<(), String> {
    Ok(())
}

/// OpenCode's config file when it lists the npm package.
fn npm_config(opencode: &Path) -> Option<PathBuf> {
    ["opencode.jsonc", "opencode.json"]
        .iter()
        .map(|name| opencode.join(name))
        .find(|path| {
            std::fs::read_to_string(path)
                .is_ok_and(|text| text.contains(&format!("\"{NPM_PACKAGE}")))
        })
}

/// `$XDG_CONFIG_HOME/opencode`, else `~/.config/opencode`.
fn opencode_dir(home: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os("OPENCODE_CONFIG_DIR") {
        return PathBuf::from(path);
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map_or_else(|| home.join(".config"), PathBuf::from)
        .join("opencode")
}

/// `$HOMEBREW_USER_CONFIG_HOME/services/chauffeur.env`, else
/// `~/.homebrew/services/chauffeur.env`, as `brew services` reads it.
fn service_env(home: &Path) -> PathBuf {
    std::env::var_os("HOMEBREW_USER_CONFIG_HOME")
        .map_or_else(|| home.join(".homebrew"), PathBuf::from)
        .join("services/chauffeur.env")
}

/// The plugin installed beside this binary.
fn installed_plugin() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|error| format!("find this binary: {error}"))?;
    let exe = exe.canonicalize().unwrap_or(exe);

    plugin_beside(&exe).ok_or_else(|| {
        format!(
            "no plugin installed beside {}; pass --plugin PATH",
            exe.display()
        )
    })
}

/// The shim for the binary at `exe`. Under a Homebrew Cellar
/// (`<prefix>/Cellar/<name>/<version>/bin`), its stable `opt` path, which
/// follows upgrades; otherwise beside the binary's own prefix.
fn plugin_beside(exe: &Path) -> Option<PathBuf> {
    let prefix = exe.parent()?.parent()?;
    let name = prefix.parent()?;
    let cellar = name.parent()?;

    if cellar.file_name().is_some_and(|folder| folder == "Cellar") {
        let opt = cellar.parent()?.join("opt").join(name.file_name()?);

        if opt.join(SHIM).is_file() {
            return Some(opt.join(SHIM));
        }
    }

    let beside = prefix.join(SHIM);

    beside.is_file().then_some(beside)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(test: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("chauffeur-setup-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    #[test]
    fn a_cellar_binary_links_the_opt_plugin_so_upgrades_follow() {
        let root = folder("cellar");
        let exe = root.join("Cellar/chauffeur/0.2.0/bin/chauffeur");

        touch(&exe);
        touch(&root.join("Cellar/chauffeur/0.2.0").join(SHIM));
        std::fs::create_dir_all(root.join("opt")).unwrap();
        std::os::unix::fs::symlink(
            root.join("Cellar/chauffeur/0.2.0"),
            root.join("opt/chauffeur"),
        )
        .unwrap();

        assert_eq!(
            plugin_beside(&exe),
            Some(root.join("opt/chauffeur").join(SHIM))
        );
    }

    #[test]
    fn another_prefix_links_the_plugin_beside_its_binary() {
        let root = folder("prefix");
        let exe = root.join("bin/chauffeur");

        touch(&exe);
        assert_eq!(plugin_beside(&exe), None);
        touch(&root.join(SHIM));
        assert_eq!(plugin_beside(&exe), Some(root.join(SHIM)));
    }

    fn options(root: &Path) -> Options {
        let plugin = root.join("opt/chauffeur").join(SHIM);

        touch(&plugin);

        Options {
            plugin,
            opencode: root.join("config/opencode"),
            service: None,
            key: None,
            force: false,
            dry_run: false,
        }
    }

    #[test]
    fn setup_links_once_and_says_so_again() {
        let root = folder("link");
        let options = options(&root);
        let link = options.opencode.join("plugins/chauffeur.js");

        setup(&options).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), options.plugin);

        let again = setup(&options).unwrap();

        assert_eq!(again.len(), 1);
        assert!(again[0].contains("already links"), "{again:?}");
    }

    #[test]
    fn a_dry_run_changes_nothing() {
        let root = folder("dry");
        let options = Options {
            dry_run: true,
            service: Some(root.join("services/chauffeur.env")),
            ..self::options(&root)
        };
        let done = setup(&options).unwrap();

        assert!(done[0].starts_with("would link"), "{done:?}");
        assert!(!options.opencode.join("plugins").exists());
        assert!(!root.join("services").exists());
    }

    #[test]
    fn an_existing_plugin_or_an_npm_install_needs_force() {
        let root = folder("force");
        let options = options(&root);
        let link = options.opencode.join("plugins/chauffeur.js");

        touch(&link);
        let error = setup(&options).unwrap_err();
        assert!(error.contains("--force"), "{error}");

        std::fs::write(
            options.opencode.join("opencode.jsonc"),
            r#"{ "plugin": ["@fdcn/chauffeur@0.2.0"] }"#,
        )
        .unwrap();
        let error = setup(&options).unwrap_err();
        assert!(error.contains("@fdcn/chauffeur"), "{error}");

        let forced = Options {
            force: true,
            ..options
        };
        assert!(setup(&forced).unwrap_err().contains("@fdcn/chauffeur"));
        std::fs::remove_file(forced.opencode.join("opencode.jsonc")).unwrap();
        setup(&forced).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), forced.plugin);
    }

    #[test]
    fn the_service_env_file_is_private_and_never_overwritten() {
        use std::os::unix::fs::PermissionsExt;

        let root = folder("service");
        let env = root.join("services/chauffeur.env");
        let options = Options {
            service: Some(env.clone()),
            key: Some("tsk-test".into()),
            ..self::options(&root)
        };
        let mode = || std::fs::metadata(&env).unwrap().permissions().mode() & 0o777;

        setup(&options).unwrap();
        assert_eq!(
            std::fs::read_to_string(&env).unwrap(),
            "TYPESAFE_API_KEY=tsk-test\n"
        );
        assert_eq!(mode(), 0o600);

        std::fs::write(
            &env,
            "TYPESAFE_API_KEY=mine\nCHAUFFEUR_IDLE_STEERING=true\n",
        )
        .unwrap();
        std::fs::set_permissions(&env, std::fs::Permissions::from_mode(0o644)).unwrap();
        setup(&options).unwrap();
        assert!(std::fs::read_to_string(&env).unwrap().contains("=mine"));
        assert_eq!(mode(), 0o600);
    }

    #[test]
    fn without_a_key_the_env_file_asks_for_one() {
        let root = folder("nokey");
        let env = root.join("services/chauffeur.env");
        let done = setup(&Options {
            service: Some(env.clone()),
            ..options(&root)
        })
        .unwrap();

        assert!(
            std::fs::read_to_string(&env)
                .unwrap()
                .starts_with("# TYPESAFE_API_KEY=")
        );
        assert!(
            done.iter()
                .any(|line| line.contains("add TYPESAFE_API_KEY")),
            "{done:?}"
        );
    }

    #[test]
    fn service_does_not_follow_symlinks_or_accept_multiline_secrets() {
        let root = folder("unsafe-env");
        let original = root.join("original");
        std::fs::write(&original, "keep me").unwrap();
        let env = root.join("linked.env");
        std::os::unix::fs::symlink(&original, &env).unwrap();
        assert!(
            service(&env, Some("secret"), false)
                .unwrap_err()
                .contains("symlink")
        );
        assert_eq!(std::fs::read_to_string(&original).unwrap(), "keep me");
        let options = Options {
            service: Some(root.join("service.env")),
            key: Some("key\nOTHER=value".into()),
            ..options(&root)
        };
        assert!(setup(&options).unwrap_err().contains("one line"));
        assert!(!options.opencode.join("plugins").exists());
    }
}
