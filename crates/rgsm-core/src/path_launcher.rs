use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::backup::Game;
use crate::config::Config;
use crate::path_pattern::StoreKind;
use crate::path_resolver::{self, PathContext};
use crate::steam::{self, InstalledSteamGame};

#[derive(Debug, Clone, PartialEq, Eq)]
enum LaunchStrategy {
    OpenWithSystem,
    RunDirectly { working_dir: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ManagedLaunchTarget {
    Filesystem(PathBuf),
    Registry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenManagedLocationOutcome {
    Opened,
    Warning(OpenManagedLocationWarning),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenManagedLocationWarning {
    RegistryOpenUnsupported,
}

/// How a Game launch must be dispatched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameLaunchDispatch {
    /// Steam owns the launch. Spawning a Steam game's executable directly skips
    /// Steam DRM, the overlay and the `SteamAppId` environment, so the game exits
    /// immediately; opening this URL lets Steam bootstrap all of it.
    Steam { app_id: String },
    /// The configured path is started directly, or opened with the system handler.
    ConfiguredPath(PathBuf),
}

pub fn open_path(path: &Path) -> Result<()> {
    match launch_strategy(path) {
        LaunchStrategy::OpenWithSystem => {
            open::that(path).with_context(|| format!("Failed to open path '{}'", path.display()))?
        }
        LaunchStrategy::RunDirectly { working_dir } => {
            Command::new(path)
                .current_dir(&working_dir)
                .spawn()
                .with_context(|| {
                    format!(
                        "Failed to launch '{}' with working directory '{}'",
                        path.display(),
                        working_dir.display()
                    )
                })?;
        }
    }

    Ok(())
}

pub fn open_managed_location(
    raw_path: &str,
    path_ctx: Option<&PathContext>,
    config: &Config,
) -> Result<OpenManagedLocationOutcome> {
    match managed_launch_target(raw_path, path_ctx, config)? {
        ManagedLaunchTarget::Filesystem(path) => {
            open_path(&path)?;
            Ok(OpenManagedLocationOutcome::Opened)
        }
        ManagedLaunchTarget::Registry => Ok(OpenManagedLocationOutcome::Warning(
            OpenManagedLocationWarning::RegistryOpenUnsupported,
        )),
    }
}

/// Decide how a Game launch must be dispatched.
///
/// The configured path stays authoritative for process detection and for every
/// non-Steam Game; only a path that belongs to a Steam install is handed to Steam.
pub fn resolve_game_launch(
    game: &Game,
    launch_path: &str,
    path_ctx: Option<&PathContext>,
    config: &Config,
) -> Result<GameLaunchDispatch> {
    let installed = steam::scan_all_installed_games().unwrap_or_default();
    resolve_game_launch_with(game, launch_path, path_ctx, config, &installed)
}

fn resolve_game_launch_with(
    game: &Game,
    launch_path: &str,
    path_ctx: Option<&PathContext>,
    config: &Config,
    installed: &HashMap<String, InstalledSteamGame>,
) -> Result<GameLaunchDispatch> {
    let ManagedLaunchTarget::Filesystem(path) =
        managed_launch_target(launch_path, path_ctx, config)?
    else {
        // Registry locations are never Steam launches; the caller keeps warning
        // about them exactly like any other managed location.
        return Ok(GameLaunchDispatch::ConfiguredPath(PathBuf::from(
            launch_path,
        )));
    };

    if let Some(app_id) = steam_app_id_for_game(game, &path, installed) {
        return Ok(GameLaunchDispatch::Steam { app_id });
    }

    Ok(GameLaunchDispatch::ConfiguredPath(path))
}

/// Steam app id that owns `launch_path`, when Steam must bootstrap the Game.
///
/// A discovered Steam install wins over the manifest id because it proves that
/// this concrete path belongs to Steam. The manifest id is only a fallback for a
/// path that still looks like a Steam install, which covers `appmanifest` files
/// that fail to parse; it is never trusted for an arbitrary executable.
pub fn steam_app_id_for_game(
    game: &Game,
    launch_path: &Path,
    installed: &HashMap<String, InstalledSteamGame>,
) -> Option<String> {
    if let Some(app_id) = steam_app_id_for_install_path(launch_path, installed) {
        return Some(app_id.to_string());
    }

    let manifest_id = game
        .ludusavi_meta
        .as_ref()
        .and_then(|meta| meta.store_game_id(StoreKind::Steam))?;
    if is_steam_install_path(launch_path) {
        return Some(manifest_id.to_string());
    }

    None
}

/// Match a launch path against the Steam installs discovered from
/// `appmanifest_*.acf`. The deepest matching install wins.
pub fn steam_app_id_for_install_path(
    launch_path: &Path,
    installed: &HashMap<String, InstalledSteamGame>,
) -> Option<u32> {
    installed
        .values()
        .filter(|game| path_is_under(launch_path, &game.install_path))
        .max_by_key(|game| game.install_path.as_os_str().len())
        .map(|game| game.app_id)
}

/// Whether a path looks like a Steam install (`…\steamapps\common\…`), which is
/// what a manifest id may be trusted for.
fn is_steam_install_path(path: &Path) -> bool {
    let normalized = normalize_for_comparison(path);
    normalized.contains(r"\steamapps\common\")
}

/// Case-insensitive containment that respects component boundaries, so
/// `…\Sekiro` never matches `…\Sekiro2`.
fn path_is_under(path: &Path, directory: &Path) -> bool {
    let normalized_path = normalize_for_comparison(path);
    let normalized_directory = normalize_for_comparison(directory);
    let normalized_directory = normalized_directory.trim_end_matches('\\');
    if normalized_directory.is_empty() {
        return false;
    }

    normalized_path == normalized_directory
        || normalized_path.starts_with(&format!("{normalized_directory}\\"))
}

fn normalize_for_comparison(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

/// Open a Steam Game so the Steam client performs the launch.
pub fn open_steam_game(app_id: &str) -> Result<()> {
    let url = steam_launch_url(app_id);
    open::that(&url).with_context(|| format!("Failed to launch Steam game {app_id}"))
}

pub fn steam_launch_url(app_id: &str) -> String {
    format!("steam://rungameid/{app_id}")
}

fn launch_strategy(path: &Path) -> LaunchStrategy {
    if should_run_directly(path) {
        let working_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        LaunchStrategy::RunDirectly { working_dir }
    } else {
        LaunchStrategy::OpenWithSystem
    }
}

fn managed_launch_target(
    raw_path: &str,
    path_ctx: Option<&PathContext>,
    config: &Config,
) -> Result<ManagedLaunchTarget> {
    if crate::backup::registry::is_registry_path(raw_path) {
        return Ok(ManagedLaunchTarget::Registry);
    }

    let path = path_resolver::resolve_path(raw_path, path_ctx, config)
        .with_context(|| format!("Failed to resolve path '{raw_path}'"))?;
    Ok(ManagedLaunchTarget::Filesystem(path))
}

#[cfg(target_os = "windows")]
fn should_run_directly(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "exe" | "com"))
            .unwrap_or(false)
}

#[cfg(unix)]
fn should_run_directly(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(any(target_os = "windows", unix)))]
fn should_run_directly(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::{
        GameLaunchDispatch, LaunchStrategy, ManagedLaunchTarget, OpenManagedLocationOutcome,
        OpenManagedLocationWarning, launch_strategy, managed_launch_target, open_managed_location,
        resolve_game_launch_with, steam_app_id_for_game, steam_app_id_for_install_path,
        steam_launch_url,
    };
    use crate::backup::{Game, LudusaviMeta, StoreGameId};
    use crate::config::Config;
    use crate::path_pattern::StoreKind;
    use crate::steam::InstalledSteamGame;
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use temp_dir::TempDir;

    fn steam_game(app_id: u32, install_dir: &str, install_path: &str) -> InstalledSteamGame {
        InstalledSteamGame {
            app_id,
            name: install_dir.to_string(),
            install_dir: install_dir.to_string(),
            install_path: PathBuf::from(install_path),
        }
    }

    fn installed(
        library_root: &str,
        install_dir: &str,
        app_id: u32,
    ) -> HashMap<String, InstalledSteamGame> {
        HashMap::from([(
            install_dir.to_ascii_lowercase(),
            steam_game(
                app_id,
                install_dir,
                &format!("{library_root}/steamapps/common/{install_dir}"),
            ),
        )])
    }

    fn game_with_steam_id(app_id: &str) -> Game {
        Game {
            name: "Game".to_string(),
            storage_key: "game".to_string(),
            save_paths: Vec::new(),
            game_paths: HashMap::new(),
            next_save_unit_id: 0,
            cloud_sync_enabled: false,
            auto_backup: None,
            ludusavi_meta: Some(LudusaviMeta {
                install_dirs: Vec::new(),
                store_game_ids: vec![StoreGameId {
                    store: StoreKind::Steam,
                    id: app_id.to_string(),
                }],
            }),
            device_bindings: HashMap::new(),
        }
    }

    fn game_without_meta() -> Game {
        Game {
            ludusavi_meta: None,
            ..game_with_steam_id("0")
        }
    }

    #[test]
    fn steam_launch_url_uses_the_rungameid_handler() {
        assert_eq!(steam_launch_url("814380"), "steam://rungameid/814380");
    }

    #[test]
    fn install_paths_inside_a_steam_library_resolve_to_the_app_id() {
        let installed = installed("M:/SteamLibrary", "Sekiro", 814380);
        let launch_path = PathBuf::from(r"M:\SteamLibrary\steamapps\common\Sekiro\sekiro.exe");

        assert_eq!(
            steam_app_id_for_install_path(&launch_path, &installed),
            Some(814380)
        );
    }

    #[test]
    fn install_path_matching_is_case_insensitive() {
        let installed = installed("M:/SteamLibrary", "Sekiro", 814380);
        let launch_path = PathBuf::from(r"m:\steamlibrary\STEAMAPPS\common\sekiro\Sekiro.exe");

        assert_eq!(
            steam_app_id_for_install_path(&launch_path, &installed),
            Some(814380)
        );
    }

    #[test]
    fn install_path_matching_respects_component_boundaries() {
        let installed = installed("M:/SteamLibrary", "Sekiro", 814380);
        let launch_path = PathBuf::from(r"M:\SteamLibrary\steamapps\common\Sekiro2\other.exe");

        assert_eq!(steam_app_id_for_install_path(&launch_path, &installed), None);
    }

    #[test]
    fn the_deepest_matching_install_wins() {
        let installed = HashMap::from([
            (
                "library".to_string(),
                steam_game(1, "library", "M:/SteamLibrary/steamapps/common"),
            ),
            (
                "sekiro".to_string(),
                steam_game(814380, "Sekiro", "M:/SteamLibrary/steamapps/common/Sekiro"),
            ),
        ]);
        let launch_path = PathBuf::from(r"M:\SteamLibrary\steamapps\common\Sekiro\sekiro.exe");

        assert_eq!(
            steam_app_id_for_install_path(&launch_path, &installed),
            Some(814380)
        );
    }

    #[test]
    fn manifest_id_is_only_trusted_for_steam_shaped_paths() {
        let game = game_with_steam_id("814380");

        assert_eq!(
            steam_app_id_for_game(
                &game,
                &PathBuf::from(r"M:\SteamLibrary\steamapps\common\Sekiro\sekiro.exe"),
                &HashMap::new()
            ),
            Some("814380".to_string())
        );
        assert_eq!(
            steam_app_id_for_game(
                &game,
                &PathBuf::from(r"D:\Repack\Sekiro\sekiro.exe"),
                &HashMap::new()
            ),
            None
        );
    }

    #[test]
    fn a_discovered_install_outranks_the_manifest_id() {
        let game = game_with_steam_id("999");
        let installed = installed("M:/SteamLibrary", "Sekiro", 814380);

        assert_eq!(
            steam_app_id_for_game(
                &game,
                &PathBuf::from(r"M:\SteamLibrary\steamapps\common\Sekiro\sekiro.exe"),
                &installed
            ),
            Some("814380".to_string())
        );
    }

    #[test]
    fn games_without_steam_information_keep_the_configured_path() {
        let game = game_without_meta();
        let launch_path = r"M:\SteamLibrary\steamapps\common\Sekiro\sekiro.exe";
        let dispatch =
            resolve_game_launch_with(&game, launch_path, None, &Config::default(), &HashMap::new())
                .unwrap();

        assert_eq!(
            dispatch,
            GameLaunchDispatch::ConfiguredPath(PathBuf::from(launch_path))
        );
    }

    #[test]
    fn steam_installs_dispatch_through_steam() {
        let game = game_without_meta();
        let launch_path = r"M:\SteamLibrary\steamapps\common\Sekiro\sekiro.exe";
        let installed = installed("M:/SteamLibrary", "Sekiro", 814380);
        let dispatch = resolve_game_launch_with(
            &game,
            launch_path,
            None,
            &Config::default(),
            &installed,
        )
        .unwrap();

        assert_eq!(
            dispatch,
            GameLaunchDispatch::Steam {
                app_id: "814380".to_string()
            }
        );
    }

    #[test]
    fn plain_executables_dispatch_through_the_configured_path() {
        let temp_dir = TempDir::new().unwrap();
        let game_dir = temp_dir.path().join("game");
        fs::create_dir(&game_dir).unwrap();
        let exe_path = game_dir.join("game.exe");
        fs::write(&exe_path, []).unwrap();

        let dispatch = resolve_game_launch_with(
            &game_without_meta(),
            exe_path.to_str().unwrap(),
            None,
            &Config::default(),
            &HashMap::new(),
        )
        .unwrap();

        assert_eq!(dispatch, GameLaunchDispatch::ConfiguredPath(exe_path));
    }

    #[test]
    fn uses_system_open_for_directories() {
        let temp_dir = TempDir::new().unwrap();
        let game_dir = temp_dir.path().join("game");
        fs::create_dir(&game_dir).unwrap();

        assert_eq!(launch_strategy(&game_dir), LaunchStrategy::OpenWithSystem);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn launches_windows_executables_from_their_parent_directory() {
        let temp_dir = TempDir::new().unwrap();
        let game_dir = temp_dir.path().join("game");
        fs::create_dir(&game_dir).unwrap();
        let exe_path = game_dir.join("Turing Complete.EXE");
        fs::write(&exe_path, []).unwrap();

        assert_eq!(
            launch_strategy(&exe_path),
            LaunchStrategy::RunDirectly {
                working_dir: game_dir
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn launches_unix_executables_from_their_parent_directory() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let game_dir = temp_dir.path().join("game");
        fs::create_dir(&game_dir).unwrap();
        let binary_path = game_dir.join("game.sh");
        fs::write(&binary_path, []).unwrap();

        let mut permissions = fs::metadata(&binary_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary_path, permissions).unwrap();

        assert_eq!(
            launch_strategy(&binary_path),
            LaunchStrategy::RunDirectly {
                working_dir: game_dir
            }
        );
    }

    #[test]
    fn keeps_non_executable_files_on_system_open_path() {
        let temp_dir = TempDir::new().unwrap();
        let shortcut_path = temp_dir.path().join("game.lnk");
        fs::write(&shortcut_path, []).unwrap();

        assert_eq!(
            launch_strategy(&shortcut_path),
            LaunchStrategy::OpenWithSystem
        );
    }

    #[test]
    fn routes_registry_locations_without_filesystem_resolution() {
        let target = managed_launch_target(
            "REGISTRY:HKEY_CURRENT_USER/Software/RGSM Test",
            None,
            &Config::default(),
        )
        .unwrap();

        assert_eq!(target, ManagedLaunchTarget::Registry);
    }

    #[test]
    fn warns_instead_of_launching_registry_locations() {
        let outcome = open_managed_location(
            "REGISTRY:HKEY_CURRENT_USER/Software/RGSM Test",
            None,
            &Config::default(),
        )
        .unwrap();

        assert_eq!(
            outcome,
            OpenManagedLocationOutcome::Warning(
                OpenManagedLocationWarning::RegistryOpenUnsupported
            )
        );
    }

    #[test]
    fn routes_filesystem_locations_through_path_resolution() {
        let target = managed_launch_target("<home>", None, &Config::default()).unwrap();

        assert!(matches!(target, ManagedLaunchTarget::Filesystem(_)));
    }
}
